//! ROI control test

use protocol::daq::{
    hardware_service_client::HardwareServiceClient,
    GetParameterRequest, SetParameterRequest, ListDevicesRequest,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("DAQ_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:50051".into());
    println!("Connecting to daemon at {}", endpoint);

    let channel = tonic::transport::Channel::from_shared(endpoint)?
        .connect_timeout(std::time::Duration::from_secs(10))
        .connect()
        .await?;

    let mut client = HardwareServiceClient::new(channel);

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

    println!("\nROI control test for camera: {}", cam);

    // Get current ROI
    println!("\n1. Getting current ROI...");
    match client.get_parameter(GetParameterRequest {
        device_id: cam.clone(),
        parameter_name: "acquisition.roi".to_string(),
    }).await {
        Ok(resp) => {
            println!("   Current ROI: {}", resp.get_ref().value);
        }
        Err(e) => {
            println!("   Error: {}", e.message());
        }
    }

    // Try setting different ROIs (format: {x, y, width, height})
    let roi_values = [
        r#"{"x": 768, "y": 768, "width": 512, "height": 512}"#,  // Center 512x512
        r#"{"x": 0, "y": 0, "width": 1024, "height": 1024}"#,    // Top-left 1024x1024
        r#"{"x": 0, "y": 0, "width": 2048, "height": 2048}"#,    // Full frame (reset)
    ];

    for roi in roi_values {
        println!("\n2. Setting ROI to {}...", roi);
        match client.set_parameter(SetParameterRequest {
            device_id: cam.clone(),
            parameter_name: "acquisition.roi".to_string(),
            value: roi.to_string(),
        }).await {
            Ok(resp) => {
                if resp.get_ref().success {
                    println!("   ✓ Success");
                } else {
                    println!("   ✗ Failed: {}", resp.get_ref().error_message);
                }
            }
            Err(e) => {
                println!("   ✗ Error: {}", e.message());
            }
        }

        // Readback
        match client.get_parameter(GetParameterRequest {
            device_id: cam.clone(),
            parameter_name: "acquisition.roi".to_string(),
        }).await {
            Ok(resp) => {
                println!("   Readback: {}", resp.get_ref().value);
            }
            Err(e) => {
                println!("   Readback error: {}", e.message());
            }
        }
    }

    println!("\n✓ ROI test complete");
    Ok(())
}
