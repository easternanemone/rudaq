//! Trigger mode test
//!
//! Tests trigger mode configuration.
//!
//! Usage:
//! ```bash
//! DAQ_DAEMON_URL=http://100.117.5.12:50051 cargo run --example trigger_test
//! ```

use protocol::daq::{
    hardware_service_client::HardwareServiceClient,
    ListParametersRequest, GetParameterRequest, SetParameterRequest,
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

    println!("\nTesting trigger modes for camera: {}", cam);

    // List all parameters to find trigger-related ones
    println!("\n1. Finding trigger-related parameters...");
    let params = client.list_parameters(ListParametersRequest {
        device_id: cam.clone(),
    }).await?;

    for p in params.get_ref().parameters.iter() {
        if p.name.contains("trigger") {
            println!("   {} ({}) - {}", p.name, p.dtype, p.description);
        }
    }

    // Get current trigger mode
    println!("\n2. Getting current trigger mode...");
    match client.get_parameter(GetParameterRequest {
        device_id: cam.clone(),
        parameter_name: "acquisition.trigger_mode".to_string(),
    }).await {
        Ok(resp) => {
            println!("   Current: {}", resp.get_ref().value);
        }
        Err(e) => {
            println!("   Error getting trigger mode: {}", e.message());
        }
    }

    // Try setting different trigger modes (PVCAM valid modes)
    let test_modes = ["EdgeTrigger", "Timed", "TriggerFirst", "Timed"];

    for mode in test_modes {
        println!("\n3. Setting trigger mode to '{}'...", mode);
        match client.set_parameter(SetParameterRequest {
            device_id: cam.clone(),
            parameter_name: "acquisition.trigger_mode".to_string(),
            value: mode.to_string(),
        }).await {
            Ok(resp) => {
                if resp.get_ref().success {
                    println!("   ✓ Success");
                } else {
                    println!("   ✗ Failed: {}", resp.get_ref().error_message);
                }
            }
            Err(e) => {
                println!("   ✗ RPC error: {}", e.message());
            }
        }

        // Verify
        match client.get_parameter(GetParameterRequest {
            device_id: cam.clone(),
            parameter_name: "acquisition.trigger_mode".to_string(),
        }).await {
            Ok(resp) => {
                println!("   Readback: {}", resp.get_ref().value);
            }
            Err(e) => {
                println!("   Readback error: {}", e.message());
            }
        }
    }

    // We end with "Timed" which is the default/internal mode

    println!("\n✓ Trigger mode test complete");
    Ok(())
}
