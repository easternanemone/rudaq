//! List all camera parameters

use protocol::daq::{
    hardware_service_client::HardwareServiceClient,
    ListParametersRequest, ListDevicesRequest,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("DAQ_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:50051".into());

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

    let params = client.list_parameters(ListParametersRequest {
        device_id: cam.clone(),
    }).await?;

    println!("All {} parameters for {}:", params.get_ref().parameters.len(), cam);
    for p in params.get_ref().parameters.iter() {
        println!("  {} ({}) - {}", p.name, p.dtype, p.description);
    }

    Ok(())
}
