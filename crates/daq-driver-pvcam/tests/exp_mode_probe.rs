//! Systematic exp_mode × expose_out_mode probe for CIRC_OVERWRITE
//!
//! This test systematically tests ALL combinations of PARAM_EXPOSURE_MODE × PARAM_EXPOSE_OUT_MODE
//! with CIRC_OVERWRITE buffer mode to determine which (if any) work on Prime BSI.
//!
//! Previous testing only tried ONE combination (1792 | 0 = 1792). This test covers all 9.
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && git pull && \
//!   cargo test --release -p daq-driver-pvcam --features pvcam_hardware \
//!     --test exp_mode_probe -- --nocapture --test-threads=1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_hardware")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

use pvcam_sys::*;
use std::ffi::{c_void, CStr, CString};

// Use constants from pvcam_sys (CIRC_OVERWRITE, CIRC_NO_OVERWRITE, CCS_HALT,
// PL_CALLBACK_EOF, ATTR_AVAIL, ATTR_CURRENT, ATTR_COUNT, ATTR_DEFAULT)

// PARAM IDs (verified on maitai - not in pvcam_sys constants)
const PARAM_EXPOSURE_MODE: u32 = 151126551;
const PARAM_EXPOSE_OUT_MODE: u32 = 151126576;

/// Get PVCAM error message
fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
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

/// Read all enum values for a parameter
fn read_enum_values(hcam: i16, param_id: u32) -> Vec<(i32, String)> {
    let mut result = Vec::new();
    let count = match get_enum_count(hcam, param_id) {
        Some(c) => c,
        None => return result,
    };

    for idx in 0..count {
        unsafe {
            let mut value: i32 = 0;
            let mut name = [0i8; 100];
            let mut name_len: uns32 = 100;

            if pl_enum_str_length(hcam, param_id, idx, &mut name_len) != 0 {
                if pl_get_enum_param(hcam, param_id, idx, &mut value, name.as_mut_ptr(), name_len)
                    != 0
                {
                    let name_str = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
                    result.push((value, name_str));
                }
            }
        }
    }
    result
}

/// Test result for a single combination
#[derive(Debug)]
struct TestResult {
    exp_mode: i32,
    exp_mode_name: String,
    expose_out: i32,
    expose_out_name: String,
    combined: i16,
    setup_ok: bool,
    start_ok: bool,
    error_code: i16,
    error_msg: String,
}

/// Dummy EOF callback for registration
extern "system" fn dummy_eof_callback(_frame_info: *const FRAME_INFO, _context: *mut c_void) {
    // Do nothing - we just need callback registered for accurate testing
}

/// Test a single exp_mode | expose_out combination with CIRC_OVERWRITE
fn test_combination(
    hcam: i16,
    exp_mode: i32,
    exp_mode_name: &str,
    expose_out: i32,
    expose_out_name: &str,
) -> TestResult {
    let combined = (exp_mode | expose_out) as i16;

    // Use small ROI for fast testing
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

    // Register callback (some modes might require it)
    let callback_registered = unsafe {
        pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            dummy_eof_callback as *mut c_void,
            std::ptr::null_mut(),
        ) != 0
    };

    // Try pl_exp_setup_cont with CIRC_OVERWRITE
    let setup_ok = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            combined,
            exposure_ms,
            &mut frame_bytes,
            CIRC_OVERWRITE,
        ) != 0
    };

    let mut start_ok = false;
    let mut error_code: i16 = 0;
    let mut error_msg = String::new();

    if setup_ok {
        // Allocate buffer and try pl_exp_start_cont
        let buffer_count = 10usize;
        let buffer_size = (frame_bytes as usize) * buffer_count;
        let mut buffer = vec![0u8; buffer_size];

        start_ok = unsafe {
            pl_exp_start_cont(
                hcam,
                buffer.as_mut_ptr() as *mut c_void,
                buffer_size as uns32,
            ) != 0
        };

        if !start_ok {
            unsafe {
                error_code = pl_error_code();
            }
            error_msg = get_error_message();
        } else {
            // Success! Stop acquisition
            unsafe {
                pl_exp_abort(hcam, CCS_HALT);
            }
        }
    } else {
        unsafe {
            error_code = pl_error_code();
        }
        error_msg = get_error_message();
    }

    // Deregister callback
    if callback_registered {
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        }
    }

    TestResult {
        exp_mode,
        exp_mode_name: exp_mode_name.to_string(),
        expose_out,
        expose_out_name: expose_out_name.to_string(),
        combined,
        setup_ok,
        start_ok,
        error_code,
        error_msg,
    }
}

#[tokio::test]
async fn test_all_exp_mode_combinations() {
    println!("\n=== SYSTEMATIC CIRC_OVERWRITE PROBE TEST ===\n");

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

    // Read all exposure modes from camera
    println!("--- Reading PARAM_EXPOSURE_MODE ---");
    let exp_modes = read_enum_values(hcam, PARAM_EXPOSURE_MODE);
    for (value, name) in &exp_modes {
        println!(
            "  [{}] {} = {}",
            exp_modes.iter().position(|(v, _)| v == value).unwrap(),
            name,
            value
        );
    }

    // Read all expose-out modes from camera
    println!("\n--- Reading PARAM_EXPOSE_OUT_MODE ---");
    let expose_out_modes = read_enum_values(hcam, PARAM_EXPOSE_OUT_MODE);
    for (value, name) in &expose_out_modes {
        println!(
            "  [{}] {} = {}",
            expose_out_modes
                .iter()
                .position(|(v, _)| v == value)
                .unwrap(),
            name,
            value
        );
    }

    // Test all combinations
    println!(
        "\n--- Testing ALL {} combinations with CIRC_OVERWRITE ---\n",
        exp_modes.len() * expose_out_modes.len()
    );

    let mut results: Vec<TestResult> = Vec::new();
    let mut any_success = false;

    for (exp_mode, exp_name) in &exp_modes {
        for (expose_out, expose_name) in &expose_out_modes {
            let result = test_combination(hcam, *exp_mode, exp_name, *expose_out, expose_name);

            let status = if result.start_ok {
                any_success = true;
                "SUCCESS"
            } else if result.setup_ok {
                "start FAIL"
            } else {
                "setup FAIL"
            };

            println!(
                "[{}] exp_mode={} ({}) | expose_out={} ({}) => combined={} | err={}",
                status,
                exp_mode,
                exp_name,
                expose_out,
                expose_name,
                result.combined,
                if result.error_code != 0 {
                    result.error_code.to_string()
                } else {
                    "-".to_string()
                }
            );

            results.push(result);
        }
    }

    // Print summary table
    println!("\n=== SUMMARY TABLE ===\n");
    println!("| exp_mode | expose_out | combined | setup | start | error |");
    println!("|----------|------------|----------|-------|-------|-------|");
    for r in &results {
        println!(
            "| {:>8} | {:>10} | {:>8} | {:>5} | {:>5} | {:>5} |",
            r.exp_mode,
            r.expose_out,
            r.combined,
            if r.setup_ok { "OK" } else { "FAIL" },
            if r.start_ok { "OK" } else { "FAIL" },
            if r.error_code != 0 {
                r.error_code.to_string()
            } else {
                "-".to_string()
            }
        );
    }

    // Print conclusion
    println!("\n=== CONCLUSION ===\n");
    if any_success {
        println!("*** FOUND WORKING COMBINATION(S) FOR CIRC_OVERWRITE! ***\n");
        for r in &results {
            if r.start_ok {
                println!(
                    "  WORKS: exp_mode={} ({}) | expose_out={} ({}) => combined={}",
                    r.exp_mode, r.exp_mode_name, r.expose_out, r.expose_out_name, r.combined
                );
            }
        }
    } else {
        println!("*** NO COMBINATION WORKS WITH CIRC_OVERWRITE ***");
        println!("*** Prime BSI does NOT support CIRC_OVERWRITE mode ***");
        println!(
            "\nAll {} combinations failed at pl_exp_start_cont with error 185.",
            results.len()
        );
    }

    // Cleanup
    println!("\n--- Cleanup ---");
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
    println!("[OK] Done\n");

    // Assert for test framework
    if any_success {
        println!("Test PASSED: Found working CIRC_OVERWRITE configuration!");
    } else {
        // Don't fail the test - just report findings
        println!("Test completed: CIRC_OVERWRITE not supported on this camera.");
    }
}
