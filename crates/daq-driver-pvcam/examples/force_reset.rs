use anyhow::Result;
use std::ffi::{CStr, CString};

#[cfg(not(feature = "pvcam_sdk"))]
fn main() {
    println!("This example requires the 'pvcam_sdk' feature.");
}

#[cfg(feature = "pvcam_sdk")]
fn main() -> Result<()> {
    use pvcam_sys::*;

    unsafe {
        println!("Initializing PVCAM...");
        if pl_pvcam_init() == 0 {
            return Err(anyhow::anyhow!("pl_pvcam_init failed: {}", get_error()));
        }

        let mut total_cameras: i16 = 0;
        if pl_cam_get_total(&mut total_cameras) == 0 {
            let err = get_error();
            pl_pvcam_uninit();
            return Err(anyhow::anyhow!("pl_cam_get_total failed: {}", err));
        }

        println!("Found {} cameras.", total_cameras);

        for i in 0..total_cameras {
            let mut name_buf = vec![0i8; 256];
            if pl_cam_get_name(i, name_buf.as_mut_ptr()) == 0 {
                println!("Failed to get name for camera index {}", i);
                continue;
            }
            let name = CStr::from_ptr(name_buf.as_ptr()).to_string_lossy();
            println!("Opening camera {}: {}", i, name);

            let mut hcam: i16 = 0;
            // Use OPEN_EXCLUSIVE (0)
            if pl_cam_open(name_buf.as_mut_ptr(), &mut hcam, 0) == 0 {
                println!("  Failed to open camera {}: {}", name, get_error());
                continue;
            }
            println!("  Camera opened. Handle: {}", hcam);

            println!("  Aborting acquisition (pl_exp_abort with CCS_HALT)...");
            if pl_exp_abort(hcam, CCS_HALT as i16) == 0 {
                println!("  pl_exp_abort failed: {}", get_error());
            } else {
                println!("  Acquisition aborted.");
            }

            // pl_exp_uninit_seq is deprecated/removed in modern PVCAM versions
            println!("  (Skipping pl_exp_uninit_seq - removed in modern SDK)");

            // Optional: reset post-processing
            println!("  Resetting PP features (pl_pp_reset)...");
            if pl_pp_reset(hcam) == 0 {
                 println!("  pl_pp_reset failed: {}", get_error());
            } else {
                 println!("  PP features reset.");
            }

            println!("  Closing camera...");
            if pl_cam_close(hcam) == 0 {
                println!("  pl_cam_close failed: {}", get_error());
            } else {
                println!("  Camera closed.");
            }
        }

        println!("Uninitializing PVCAM...");
        pl_pvcam_uninit();
        println!("Done.");
    }

    Ok(())
}

#[cfg(feature = "pvcam_sdk")]
unsafe fn get_error() -> String {
    use pvcam_sys::*;
    let code = pl_error_code();
    let mut msg_buf = vec![0i8; 256];
    pl_error_message(code, msg_buf.as_mut_ptr());
    format!("{} (code {})", CStr::from_ptr(msg_buf.as_ptr()).to_string_lossy(), code)
}
