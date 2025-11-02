#![cfg(feature = "pvcam_hardware")]

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use rust_daq::instruments_v2::pvcam_sdk::{PvcamParam, RealPvcamSdk, TriggerMode};
use tokio::time::timeout;

/// PVCAM hardware smoke test.
///
/// Run with:
/// `PVCAM_SMOKE_TEST=1 PVCAM_CAMERA_NAME=PrimeBSI cargo test --test pvcam_hardware_smoke --features pvcam_hardware -- --nocapture`
#[tokio::test(flavor = "multi_thread")]
async fn pvcam_hardware_smoke() -> Result<()> {
    if std::env::var("PVCAM_SMOKE_TEST").unwrap_or_default() != "1" {
        eprintln!(
            "Skipping pvcam_hardware_smoke (set PVCAM_SMOKE_TEST=1 to enable real camera check)"
        );
        return Ok(());
    }

    let sdk = Arc::new(RealPvcamSdk::new());
    sdk.init().context("pvcam init")?;

    let cameras = sdk
        .enumerate_cameras()
        .context("enumerate cameras")?;
    let camera_name = std::env::var("PVCAM_CAMERA_NAME")
        .ok()
        .or_else(|| cameras.first().cloned())
        .context("no PVCAM cameras found; set PVCAM_CAMERA_NAME")?;

    let handle = sdk
        .open_camera(&camera_name)
        .with_context(|| format!("open camera {camera_name}"))?;

    // Apply a quick configuration before streaming.
    sdk.set_param_u16(&handle, PvcamParam::Exposure, 100)?; // 100 ms
    sdk.set_param_u16(&handle, PvcamParam::Gain, 1)?;
    sdk.set_param_u16(
        &handle,
        PvcamParam::ExposureMode,
        TriggerMode::Timed.as_u16(),
    )?;

    let (mut frame_rx, guard) = Arc::clone(&sdk)
        .start_acquisition(handle)
        .context("start acquisition")?;

    let frame = timeout(Duration::from_secs(2), frame_rx.recv())
        .await
        .context("timeout waiting for frame")?
        .context("frame channel closed")?;

    assert!(!frame.data.is_empty(), "frame payload empty");


    drop(guard); // ensure hardware stream stops before explicit shutdown
    sdk.stop_acquisition(handle)
        .context("stop acquisition")?;
    sdk.close_camera(handle)
        .context("close camera")?;
    sdk.uninit().context("pvcam uninit")?;

    Ok(())
}
