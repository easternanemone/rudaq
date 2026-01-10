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
use std::alloc::{alloc_zeroed, dealloc, Layout};
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
const PARAM_EXPOSE_OUT_MODE: u32 = 151126576;  // Verified from bindings.rs on maitai (was 151126585)
const PARAM_SER_SIZE: u32 = 100794426;         // from bindings.rs
const PARAM_PAR_SIZE: u32 = 100794425;         // from bindings.rs
// BuildSpeedTable parameters - SDK sets these to defaults before acquisition
const PARAM_READOUT_PORT: u32 = 151126263;     // Verified on maitai
const PARAM_SPDTAB_INDEX: u32 = 16908801;      // Verified on maitai
const PARAM_GAIN_INDEX: u32 = 16908800;        // Verified on maitai
const PARAM_BIT_DEPTH: u32 = 16908799;         // Verified on maitai
// PARAM_FRAME_BUFFER_SIZE - available AFTER setup_cont, gives min/max/recommended buffer sizes
const PARAM_FRAME_BUFFER_SIZE: u32 = 184746284; // ((3<<16) + (11<<24) + 300) = 184746284

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
                println!("    [WARN] Failed to set PARAM_READOUT_PORT: {}", get_error_message());
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
                println!("    [WARN] Failed to set PARAM_SPDTAB_INDEX: {}", get_error_message());
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
                println!("    [WARN] Failed to set PARAM_GAIN_INDEX: {}", get_error_message());
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
            if pl_get_param(hcam, PARAM_BIT_DEPTH, ATTR_CURRENT, &mut bit_depth as *mut _ as *mut c_void) != 0 {
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
        const FRAME_COUNT: usize = 50;  // SDK default

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
                println!("[OK] pl_exp_setup_cont succeeded, frame_bytes = {}", frame_bytes);

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                // Round up to 4KB boundary
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout = Layout::from_size_align(aligned_size, ALIGN_4K)
                    .expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if buffer_ptr.is_null() {
                    println!("[FAIL] Failed to allocate 4KB-aligned buffer");
                } else {
                    let ptr_val = buffer_ptr as usize;
                    println!("     Buffer allocated: {} bytes at 0x{:X}", aligned_size, ptr_val);
                    println!("     Alignment check: 0x{:X} % 4096 = {}", ptr_val, ptr_val % ALIGN_4K);

                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,  // Original size, not padded
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
                println!("[WARN] Failed to register callback: {}", get_error_message());
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

                let layout = Layout::from_size_align(aligned_size, ALIGN_4K)
                    .expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,
                    );

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
            s2: 2047,  // 2048 pixels wide (0-2047)
            sbin: 1,
            p1: 0,
            p2: 2047,  // 2048 pixels tall (0-2047)
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
                println!("[OK] pl_exp_setup_cont succeeded, frame_bytes = {}", frame_bytes);

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout = Layout::from_size_align(aligned_size, ALIGN_4K)
                    .expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,
                    );

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
    println!("     Setting PARAM_READOUT_PORT, PARAM_SPDTAB_INDEX, PARAM_GAIN_INDEX to defaults...\n");

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
                println!("[OK] pl_exp_setup_cont succeeded, frame_bytes = {}", frame_bytes);

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout = Layout::from_size_align(aligned_size, ALIGN_4K)
                    .expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,
                    );

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
        const FRAME_COUNT: usize = 100;  // Use 100 frames like PVCamTestCli --buffer-frames=100

        let region = rgn_type {
            s1: 0,
            s2: 511,  // 512x512 for quick test
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
                println!("[OK] Step 1: pl_exp_setup_cont succeeded, frame_bytes = {}", frame_bytes);

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
                    println!("[WARN] Failed to register callback: {}", get_error_message());
                }

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout = Layout::from_size_align(aligned_size, ALIGN_4K)
                    .expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    // Step 3: START (after callback registration)
                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,
                    );

                    if start_result != 0 {
                        println!("[OK] Step 3: pl_exp_start_cont SUCCEEDED!");
                        println!("\n*********************************************");
                        println!("*** SOLUTION FOUND: CALLBACK ORDER MATTERS! ***");
                        println!("*** Register callback AFTER setup_cont! ***");
                        println!("*********************************************\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!("[FAIL] Step 3: pl_exp_start_cont failed: {}", get_error_message());
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
                println!("[FAIL] Step 1: pl_exp_setup_cont failed: {}", get_error_message());
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
                TIMED_MODE,  // Explicit 0, like PVCamTestCli default
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

                let layout = Layout::from_size_align(aligned_size, ALIGN_4K)
                    .expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    // Step 3: START
                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,
                    );

                    if start_result != 0 {
                        println!("[OK] Step 3: TIMED_MODE + SDK order WORKS!");
                        println!("\n*** TIMED_MODE with correct callback order works! ***\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!("[FAIL] Step 3: pl_exp_start_cont failed: {}", get_error_message());
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                }

                pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            } else {
                println!("[FAIL] Step 1: pl_exp_setup_cont failed: {}", get_error_message());
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
    unsafe { pl_cam_close(hcam); }
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
                println!("[WARN] Failed to create frame_info_struct: {}", get_error_message());
            }

            // Now try CIRC_OVERWRITE with fresh state
            const ALIGN_4K: usize = 4096;
            const FRAME_COUNT: usize = 100;

            let region = rgn_type {
                s1: 0,
                s2: 2047,  // Full frame
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
                20,  // 20ms exposure
                &mut frame_bytes,
                CIRC_OVERWRITE,
            );

            if setup_result != 0 {
                println!("[OK] Step 1: pl_exp_setup_cont succeeded, frame_bytes = {}", frame_bytes);

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
                        println!("[FAIL] Step 3: pl_exp_start_cont failed: {}", get_error_message());
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                }

                pl_cam_deregister_callback(hcam_fresh, PL_CALLBACK_EOF);
            } else {
                println!("[FAIL] Step 1: pl_exp_setup_cont failed: {}", get_error_message());
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
            println!("[OK] Camera opened for sequence mode test, handle = {}", hcam_seq);

            // Create frame_info_struct
            let mut frame_info: *mut FRAME_INFO = ptr::null_mut();
            let _ = pl_create_frame_info_struct(&mut frame_info);

            const FRAME_COUNT: uns16 = 3;  // Capture 3 frames
            const EXPOSURE_MS: uns32 = 100;  // Longer exposure to ensure completion

            let region = rgn_type {
                s1: 0,
                s2: 255,  // 256x256 for quick test
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
                1,  // region count
                &region as *const _,
                EXT_TRIG_INTERNAL,  // Camera's default mode (1792)
                EXPOSURE_MS,
                &mut buffer_bytes,
            );

            if setup_result != 0 {
                println!("[OK] pl_exp_setup_seq with EXT_TRIG_INTERNAL succeeded, buffer_bytes = {}", buffer_bytes);

                // Allocate buffer for all frames
                let mut buffer = vec![0u8; buffer_bytes as usize];

                // Start sequence acquisition
                let start_result = pl_exp_start_seq(
                    hcam_seq,
                    buffer.as_mut_ptr() as *mut c_void,
                );

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
                            println!("     Status changed: {} -> {} (bytes: {})", last_status, status, bytes_arrived);
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
            println!("[OK] Camera opened for TIMED_MODE sequence test, handle = {}", hcam_seq2);

            // Create frame_info_struct
            let mut frame_info: *mut FRAME_INFO = ptr::null_mut();
            let _ = pl_create_frame_info_struct(&mut frame_info);

            const FRAME_COUNT: uns16 = 1;  // Single frame for simplicity
            const EXPOSURE_MS: uns32 = 200;  // 200ms single exposure

            let region = rgn_type {
                s1: 0,
                s2: 255,  // 256x256
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
                1,  // region count
                &region as *const _,
                TIMED_MODE,  // 0 = immediate software-triggered
                EXPOSURE_MS,
                &mut buffer_bytes,
            );

            if setup_result != 0 {
                println!("[OK] pl_exp_setup_seq with TIMED_MODE succeeded, buffer_bytes = {}", buffer_bytes);

                // Allocate buffer
                let mut buffer = vec![0u8; buffer_bytes as usize];

                // Start sequence acquisition
                let start_result = pl_exp_start_seq(
                    hcam_seq2,
                    buffer.as_mut_ptr() as *mut c_void,
                );

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
                            println!("     Status: {} ({}) - elapsed: {:?}, bytes: {}",
                                status, status_name, start_time.elapsed(), bytes_arrived);
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
                println!("[OK] Setup for single frame, buffer_bytes = {}", buffer_bytes);

                let mut buffer = vec![0u8; buffer_bytes as usize];

                if pl_exp_start_seq(hcam_seq3, buffer.as_mut_ptr() as *mut c_void) != 0 {
                    println!("[OK] Acquisition started");

                    // Wait longer for 500ms exposure
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut last_status: i16 = -1;
                    let start_time = std::time::Instant::now();

                    for _ in 0..200 {  // 2 seconds max
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
                            println!("     {:?}: status={} ({}) bytes={}",
                                start_time.elapsed(), status, status_name, bytes_arrived);
                            last_status = status;
                        }

                        if status == 3 {  // READOUT_COMPLETE
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
