//! Minimal example showing how to enable the Arrow tap and receive frames as Arrow arrays.
//! Requires `--features arrow_tap,pvcam_hardware` and PVCAM env vars set.
//!
//! Run (on hardware host):
//! PVCAM_SDK_DIR=/opt/pvcam/sdk \
//! PVCAM_LIB_DIR=/opt/pvcam/library/x86_64 \
//! PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode \
//! LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH \
//! cargo run -p daq-driver-pvcam --example arrow_tap --features "pvcam_hardware,arrow_tap" -- PrimeBSI

use arrow::array::Array;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let camera_name = std::env::args().nth(1).unwrap_or_else(|| "PrimeBSI".to_string());
    let driver = daq_driver_pvcam::PvcamDriver::new_async(camera_name).await?;

    // Create tap channel
    let (tx, mut rx) = mpsc::channel(4);
    driver.set_arrow_tap(tx).await;

    // Start one-shot frame to exercise tap
    let frame = driver.acquire_frame().await?;
    println!("Acquired frame {}x{}", frame.width, frame.height);

    // Pull one Arrow array from tap
    if let Some(arr) = rx.recv().await {
        println!("Arrow tap received len={} nulls={}", arr.len(), arr.null_count());
        // Print a few sample pixels
        for i in 0..std::cmp::min(5, arr.len()) {
            println!("pixel[{i}] = {}", arr.value(i));
        }
    } else {
        println!("Arrow tap channel closed without data");
    }

    Ok(())
}
