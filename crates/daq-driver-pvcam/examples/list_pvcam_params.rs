//! List all available PVCAM parameters from connected camera.
//!
//! Run with: cargo run --features "pvcam_hardware" --example list_pvcam_params

use anyhow::Result;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;

#[cfg(feature = "pvcam_hardware")]
fn main() -> Result<()> {
    use std::ffi::CStr;

    println!("=== PVCAM Parameter Discovery ===\n");

    // Initialize SDK
    unsafe {
        if pl_pvcam_init() == 0 {
            eprintln!("Failed to init PVCAM");
            return Ok(());
        }
    }

    // Get camera count
    let mut total: i16 = 0;
    unsafe {
        if pl_cam_get_total(&mut total) == 0 || total == 0 {
            eprintln!("No cameras found");
            pl_pvcam_uninit();
            return Ok(());
        }
    }
    println!("Found {} camera(s)\n", total);

    // Get first camera name and open it
    let mut name_buf = [0i8; 256];
    unsafe {
        pl_cam_get_name(0, name_buf.as_mut_ptr());
    }
    let cam_name = unsafe { CStr::from_ptr(name_buf.as_ptr()).to_string_lossy() };
    println!("Camera: {}\n", cam_name);

    let mut hcam: i16 = 0;
    unsafe {
        if pl_cam_open(name_buf.as_mut_ptr(), &mut hcam, 0) == 0 {
            eprintln!("Failed to open camera");
            pl_pvcam_uninit();
            return Ok(());
        }
    }

    // List of parameters to check
    let params: Vec<(&str, u32)> = vec![
        // Camera Info
        ("PARAM_CHIP_NAME", PARAM_CHIP_NAME),
        ("PARAM_SYSTEM_NAME", PARAM_SYSTEM_NAME),
        ("PARAM_VENDOR_NAME", PARAM_VENDOR_NAME),
        ("PARAM_PRODUCT_NAME", PARAM_PRODUCT_NAME),
        ("PARAM_CAMERA_PART_NUMBER", PARAM_CAMERA_PART_NUMBER),
        ("PARAM_HEAD_SER_NUM_ALPHA", PARAM_HEAD_SER_NUM_ALPHA),
        ("PARAM_CAM_FW_VERSION", PARAM_CAM_FW_VERSION),

        // Sensor Size
        ("PARAM_SER_SIZE", PARAM_SER_SIZE),
        ("PARAM_PAR_SIZE", PARAM_PAR_SIZE),
        ("PARAM_PIX_SER_SIZE", PARAM_PIX_SER_SIZE),
        ("PARAM_PIX_PAR_SIZE", PARAM_PIX_PAR_SIZE),
        ("PARAM_PIX_SER_DIST", PARAM_PIX_SER_DIST),
        ("PARAM_PIX_PAR_DIST", PARAM_PIX_PAR_DIST),

        // Thermal
        ("PARAM_TEMP", PARAM_TEMP),
        ("PARAM_TEMP_SETPOINT", PARAM_TEMP_SETPOINT),
        ("PARAM_COOLING_MODE", PARAM_COOLING_MODE),
        ("PARAM_FAN_SPEED_SETPOINT", PARAM_FAN_SPEED_SETPOINT),

        // Timing
        ("PARAM_READOUT_TIME", PARAM_READOUT_TIME),
        ("PARAM_CLEARING_TIME", PARAM_CLEARING_TIME),
        ("PARAM_PRE_TRIGGER_DELAY", PARAM_PRE_TRIGGER_DELAY),
        ("PARAM_POST_TRIGGER_DELAY", PARAM_POST_TRIGGER_DELAY),
        ("PARAM_CLEAR_CYCLES", PARAM_CLEAR_CYCLES),
        ("PARAM_CLEAR_MODE", PARAM_CLEAR_MODE),

        // Readout
        ("PARAM_READOUT_PORT", PARAM_READOUT_PORT),
        ("PARAM_SPDTAB_INDEX", PARAM_SPDTAB_INDEX),
        ("PARAM_PIX_TIME", PARAM_PIX_TIME),
        ("PARAM_BIT_DEPTH", PARAM_BIT_DEPTH),
        ("PARAM_GAIN_INDEX", PARAM_GAIN_INDEX),
        ("PARAM_ACTUAL_GAIN", PARAM_ACTUAL_GAIN),
        ("PARAM_READ_NOISE", PARAM_READ_NOISE),

        // Gain Multiplication
        ("PARAM_GAIN_MULT_FACTOR", PARAM_GAIN_MULT_FACTOR),
        ("PARAM_GAIN_MULT_ENABLE", PARAM_GAIN_MULT_ENABLE),

        // Exposure & Triggering
        ("PARAM_EXPOSURE_MODE", PARAM_EXPOSURE_MODE),
        ("PARAM_EXPOSE_OUT_MODE", PARAM_EXPOSE_OUT_MODE),

        // Scan Modes
        ("PARAM_SCAN_MODE", PARAM_SCAN_MODE),
        ("PARAM_SCAN_DIRECTION", PARAM_SCAN_DIRECTION),
        ("PARAM_SCAN_LINE_DELAY", PARAM_SCAN_LINE_DELAY),
        ("PARAM_SCAN_LINE_TIME", PARAM_SCAN_LINE_TIME),
        ("PARAM_SCAN_WIDTH", PARAM_SCAN_WIDTH),

        // Binning
        ("PARAM_BINNING_SER", PARAM_BINNING_SER),
        ("PARAM_BINNING_PAR", PARAM_BINNING_PAR),

        // Shutter
        ("PARAM_SHTR_STATUS", PARAM_SHTR_STATUS),
        ("PARAM_SHTR_OPEN_MODE", PARAM_SHTR_OPEN_MODE),
        ("PARAM_SHTR_OPEN_DELAY", PARAM_SHTR_OPEN_DELAY),
        ("PARAM_SHTR_CLOSE_DELAY", PARAM_SHTR_CLOSE_DELAY),

        // Image Format
        ("PARAM_IMAGE_FORMAT", PARAM_IMAGE_FORMAT),
        ("PARAM_IMAGE_COMPRESSION", PARAM_IMAGE_COMPRESSION),

        // Host Processing
        ("PARAM_HOST_FRAME_ROTATE", PARAM_HOST_FRAME_ROTATE),
        ("PARAM_HOST_FRAME_FLIP", PARAM_HOST_FRAME_FLIP),
        ("PARAM_HOST_FRAME_SUMMING_ENABLED", PARAM_HOST_FRAME_SUMMING_ENABLED),
        ("PARAM_HOST_FRAME_SUMMING_COUNT", PARAM_HOST_FRAME_SUMMING_COUNT),

        // Post-Processing
        ("PARAM_PP_INDEX", PARAM_PP_INDEX),
        ("PARAM_PP_FEAT_NAME", PARAM_PP_FEAT_NAME),
        ("PARAM_PP_FEAT_ID", PARAM_PP_FEAT_ID),
        ("PARAM_PP_PARAM_INDEX", PARAM_PP_PARAM_INDEX),
        ("PARAM_PP_PARAM_NAME", PARAM_PP_PARAM_NAME),
        ("PARAM_PP_PARAM", PARAM_PP_PARAM),

        // Metadata & Centroids
        ("PARAM_METADATA_ENABLED", PARAM_METADATA_ENABLED),
        ("PARAM_CENTROIDS_ENABLED", PARAM_CENTROIDS_ENABLED),
        ("PARAM_CENTROIDS_RADIUS", PARAM_CENTROIDS_RADIUS),
        ("PARAM_CENTROIDS_COUNT", PARAM_CENTROIDS_COUNT),
        ("PARAM_CENTROIDS_MODE", PARAM_CENTROIDS_MODE),
        ("PARAM_CENTROIDS_BG_COUNT", PARAM_CENTROIDS_BG_COUNT),
        ("PARAM_CENTROIDS_THRESHOLD", PARAM_CENTROIDS_THRESHOLD),

        // Sensor Capabilities
        ("PARAM_FWELL_CAPACITY", PARAM_FWELL_CAPACITY),
        ("PARAM_FRAME_CAPABLE", PARAM_FRAME_CAPABLE),
        ("PARAM_ACCUM_CAPABLE", PARAM_ACCUM_CAPABLE),

        // ADC
        ("PARAM_ADC_OFFSET", PARAM_ADC_OFFSET),

        // I/O
        ("PARAM_IO_ADDR", PARAM_IO_ADDR),
        ("PARAM_IO_TYPE", PARAM_IO_TYPE),
        ("PARAM_IO_DIRECTION", PARAM_IO_DIRECTION),
        ("PARAM_IO_STATE", PARAM_IO_STATE),
        ("PARAM_IO_BITDEPTH", PARAM_IO_BITDEPTH),

        // Triggering
        ("PARAM_LAST_MUXED_SIGNAL", PARAM_LAST_MUXED_SIGNAL),
        ("PARAM_TRIGTAB_SIGNAL", PARAM_TRIGTAB_SIGNAL),
        ("PARAM_TRIGB_MAX_TIMING_MODE", PARAM_TRIGB_MAX_TIMING_MODE),
        ("PARAM_EXP_RES", PARAM_EXP_RES),
        ("PARAM_EXP_RES_INDEX", PARAM_EXP_RES_INDEX),
        ("PARAM_EXP_TIME", PARAM_EXP_TIME),
    ];

    println!("=== Available Parameters ===\n");

    let mut available_count = 0;
    let mut unavailable_count = 0;

    for (name, param_id) in &params {
        let mut avail: rs_bool = 0;
        unsafe {
            if pl_get_param(hcam, *param_id, ATTR_AVAIL as i16, &mut avail as *mut _ as *mut _) != 0 && avail != 0 {
                available_count += 1;

                // Try to get value based on type
                let mut param_type: i16 = 0;
                if pl_get_param(hcam, *param_id, ATTR_TYPE as i16, &mut param_type as *mut _ as *mut _) != 0 {
                    match param_type as u32 {
                        TYPE_INT8 | TYPE_INT16 | TYPE_INT32 => {
                            let mut val: i32 = 0;
                            if pl_get_param(hcam, *param_id, ATTR_CURRENT as i16, &mut val as *mut _ as *mut _) != 0 {
                                println!("[AVAIL] {} = {} (int)", name, val);
                            } else {
                                println!("[AVAIL] {} (read error)", name);
                            }
                        }
                        TYPE_UNS8 | TYPE_UNS16 | TYPE_UNS32 => {
                            let mut val: u32 = 0;
                            if pl_get_param(hcam, *param_id, ATTR_CURRENT as i16, &mut val as *mut _ as *mut _) != 0 {
                                println!("[AVAIL] {} = {} (uint)", name, val);
                            } else {
                                println!("[AVAIL] {} (read error)", name);
                            }
                        }
                        TYPE_FLT64 => {
                            let mut val: f64 = 0.0;
                            if pl_get_param(hcam, *param_id, ATTR_CURRENT as i16, &mut val as *mut _ as *mut _) != 0 {
                                println!("[AVAIL] {} = {} (float)", name, val);
                            } else {
                                println!("[AVAIL] {} (read error)", name);
                            }
                        }
                        TYPE_BOOLEAN => {
                            let mut val: rs_bool = 0;
                            if pl_get_param(hcam, *param_id, ATTR_CURRENT as i16, &mut val as *mut _ as *mut _) != 0 {
                                println!("[AVAIL] {} = {} (bool)", name, val != 0);
                            } else {
                                println!("[AVAIL] {} (read error)", name);
                            }
                        }
                        TYPE_CHAR_PTR => {
                            let mut buf = [0i8; 256];
                            if pl_get_param(hcam, *param_id, ATTR_CURRENT as i16, buf.as_mut_ptr() as *mut _) != 0 {
                                let s = CStr::from_ptr(buf.as_ptr()).to_string_lossy();
                                println!("[AVAIL] {} = \"{}\" (string)", name, s);
                            } else {
                                println!("[AVAIL] {} (read error)", name);
                            }
                        }
                        TYPE_ENUM => {
                            let mut val: i32 = 0;
                            let mut count: u32 = 0;
                            if pl_get_param(hcam, *param_id, ATTR_CURRENT as i16, &mut val as *mut _ as *mut _) != 0 {
                                let _ = pl_get_param(hcam, *param_id, ATTR_COUNT as i16, &mut count as *mut _ as *mut _);
                                println!("[AVAIL] {} = {} (enum, {} choices)", name, val, count);
                            } else {
                                println!("[AVAIL] {} (read error)", name);
                            }
                        }
                        _ => {
                            println!("[AVAIL] {} (type={})", name, param_type);
                        }
                    }
                } else {
                    println!("[AVAIL] {} (type unknown)", name);
                }
            } else {
                unavailable_count += 1;
                println!("[N/A]   {}", name);
            }
        }
    }

    println!("\n=== Summary ===");
    println!("Available: {}", available_count);
    println!("Unavailable: {}", unavailable_count);
    println!("Total checked: {}", params.len());

    // Cleanup
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    Ok(())
}

#[cfg(not(feature = "pvcam_hardware"))]
fn main() {
    println!("This example requires the pvcam_hardware feature.");
    println!("Run with: cargo run --features pvcam_hardware --example list_pvcam_params");
}
