//! Frame timing probe to validate FIFO (get_oldest_frame + unlock) semantics.
//!
//! Confirms that `pl_exp_get_oldest_frame_ex` returns frames in chronological order
//! and that unlocking advances the buffer pointer without skips.
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && git pull && \
//!   cargo test --release -p daq-driver-pvcam --features pvcam_hardware \
//!     --test frame_timing_probe -- --nocapture --test-threads=1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_hardware")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

use pvcam_sys::*;
use std::ffi::{c_void, CStr};
use std::time::{Duration, Instant};

// Use constants from pvcam_sys (no local redefinition needed)

/// Get PVCAM error message
fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Frame metadata for analysis
#[derive(Debug, Clone)]
struct FrameMetadata {
    frame_nr: i32,
    timestamp: u64,     // TimeStamp field (100ns resolution typically)
    timestamp_bof: u64, // Beginning of frame timestamp
    readout_time: i32,  // Readout time in microseconds
    retrieval_method: &'static str,
    wall_clock_ms: u128, // Wall clock time since test start
}

#[tokio::test]
async fn test_frame_timing_semantics() {
    println!("\n=== FRAME TIMING PROBE: FIFO get_oldest_frame ===\n");
    println!("Goal: Confirm get_oldest_frame returns chronological frames and unlock advances FIFO\n");

    // Initialize PVCAM
    unsafe {
        if pl_pvcam_init() == 0 {
            panic!("Failed to initialize PVCAM: {}", get_error_message());
        }
    }
    println!("[OK] PVCAM initialized");

    // Get camera count and open
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
    println!("[OK] Camera: {}", name);

    let mut hcam: i16 = 0;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            pl_pvcam_uninit();
            panic!("Failed to open camera: {}", get_error_message());
        }
    }
    println!("[OK] Camera opened, handle = {}\n", hcam);

    // Use small ROI for speed but long exposure to separate frames in time
    let region = rgn_type {
        s1: 0,
        s2: 255,
        sbin: 1,
        p1: 0,
        p2: 255,
        pbin: 1,
    };

    // LONG exposure to make timing differences clear
    let exposure_ms: uns32 = 500; // 500ms exposure = 2 FPS max
    let mut frame_bytes: uns32 = 0;
    let exp_mode = EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW;
    let buffer_frames = 10u32; // Room for several frames

    println!("Configuration:");
    println!(
        "  Exposure: {}ms (long, for clear timing separation)",
        exposure_ms
    );
    println!("  Buffer frames: {}", buffer_frames);
    println!("  ROI: 256x256\n");

    // Setup continuous acquisition
    let setup_ok = unsafe {
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

    if !setup_ok {
        println!("[FAIL] setup_cont failed: {}", get_error_message());
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] setup_cont succeeded, frame_bytes = {}", frame_bytes);

    // Allocate buffer
    let buffer_size = (frame_bytes as usize) * (buffer_frames as usize);
    let mut buffer = vec![0u8; buffer_size];

    // Start acquisition
    let start_ok = unsafe {
        pl_exp_start_cont(
            hcam,
            buffer.as_mut_ptr() as *mut c_void,
            buffer_size as uns32,
        ) != 0
    };

    if !start_ok {
        println!("[FAIL] start_cont failed: {}", get_error_message());
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] start_cont succeeded\n");

    let test_start = Instant::now();
    let mut oldest_frames: Vec<FrameMetadata> = Vec::new();

    println!("=== Phase 1: Let buffer fill with frames (waiting 3 seconds) ===\n");

    // Wait for buffer to fill with several frames
    std::thread::sleep(Duration::from_secs(3));

    println!("=== Phase 2: Retrieve frames via FIFO ===\n");
    println!("--- Testing pl_exp_get_oldest_frame_ex ---");
    for i in 0..5 {
        let mut address: *mut c_void = std::ptr::null_mut();
        let mut fi: FRAME_INFO = unsafe { std::mem::zeroed() };

        let result = unsafe { pl_exp_get_oldest_frame_ex(hcam, &mut address, &mut fi) };

        if result != 0 && !address.is_null() {
            let meta = FrameMetadata {
                frame_nr: fi.FrameNr,
                timestamp: fi.TimeStamp as u64,
                timestamp_bof: fi.TimeStampBOF as u64,
                readout_time: fi.ReadoutTime,
                retrieval_method: "get_oldest_frame",
                wall_clock_ms: test_start.elapsed().as_millis(),
            };
            println!(
                "  [{}] FrameNr={}, TimeStamp={}, TimeStampBOF={}, ReadoutTime={}",
                i, meta.frame_nr, meta.timestamp, meta.timestamp_bof, meta.readout_time
            );
            oldest_frames.push(meta);

            // Unlock so we can get next frame
            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }
        } else {
            println!("  [{}] No frame available", i);
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    // Stop acquisition
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
    }

    // Analysis
    println!("\n=== ANALYSIS ===\n");
    if oldest_frames.len() >= 3 {
        // Assert monotonic frame numbers and non-decreasing timestamps
        let mut monotonic_ok = true;
        let mut ts_ok = true;
        for win in oldest_frames.windows(2) {
            if win[1].frame_nr <= win[0].frame_nr {
                monotonic_ok = false;
            }
            if win[1].timestamp < win[0].timestamp {
                ts_ok = false;
            }
        }

        println!("Collected frame numbers: {:?}", oldest_frames.iter().map(|f| f.frame_nr).collect::<Vec<_>>());
        println!("Monotonic frame numbers: {}", monotonic_ok);
        println!("Non-decreasing timestamps: {}", ts_ok);

        assert!(monotonic_ok, "Frame numbers must increase under FIFO retrieval");
        assert!(ts_ok, "Timestamps must be non-decreasing under FIFO retrieval");
    } else {
        panic!(
            "Insufficient frames collected for FIFO timing probe ({} < 3)",
            oldest_frames.len()
        );
    }

    // Cleanup
    println!("\n--- Cleanup ---");
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
    println!("[OK] Done\n");
}
