//! Exposure control test
//!
//! Tests setting exposure time and verifying it takes effect.
//!
//! Usage:
//! ```bash
//! DAQ_DAEMON_URL=http://100.117.5.12:50051 cargo run --example exposure_test
//! ```

use protocol::daq::{
    hardware_service_client::HardwareServiceClient, SetExposureRequest, GetExposureRequest,
    ListDevicesRequest,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("DAQ_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:50051".into());
    println!("Connecting to daemon at {}", endpoint);

    let channel = tonic::transport::Channel::from_shared(endpoint.clone())?
        .connect_timeout(std::time::Duration::from_secs(10))
        .connect()
        .await?;

    let mut client = HardwareServiceClient::new(channel);

    // Find camera
    let devices = client.list_devices(ListDevicesRequest {
        capability_filter: None,
    }).await?;

    let camera_id = devices.get_ref().devices.iter()
        .find(|d| d.is_frame_producer || d.id == "camera")
        .map(|d| d.id.clone());

    let Some(cam) = camera_id else {
        println!("No camera found");
        return Ok(());
    };

    println!("\nTesting exposure control for camera: {}", cam);

    // Get current exposure
    println!("\n1. Getting current exposure...");
    let current = client.get_exposure(GetExposureRequest {
        device_id: cam.clone(),
    }).await?;
    let current_exposure = current.get_ref();
    println!("   Current exposure: {:.2} ms", current_exposure.exposure_ms);

    // Try setting different exposures
    let test_exposures = [10.0, 50.0, 100.0, 20.0];

    for target_ms in test_exposures {
        println!("\n2. Setting exposure to {:.1} ms...", target_ms);
        match client.set_exposure(SetExposureRequest {
            device_id: cam.clone(),
            exposure_ms: target_ms,
        }).await {
            Ok(resp) => {
                let result = resp.get_ref();
                if result.success {
                    println!("   ✓ Success! Actual exposure: {:.2} ms", result.actual_exposure_ms);
                } else {
                    println!("   ✗ Failed: {}", result.error_message);
                }
            }
            Err(e) => {
                println!("   ✗ RPC error: {}", e.message());
            }
        }

        // Verify by reading back
        let verify = client.get_exposure(GetExposureRequest {
            device_id: cam.clone(),
        }).await?;
        println!("   Readback: {:.2} ms", verify.get_ref().exposure_ms);
    }

    println!("\n✓ Exposure control test complete");
    Ok(())
}
