//! gRPC Client for rust-daq daemon
//!
//! Provides a high-level interface to the daemon's gRPC services.

use anyhow::{anyhow, Result};
use rust_daq::grpc::{
    HardwareServiceClient, ListDevicesRequest, MoveRequest, ReadValueRequest,
    StopMotionRequest, StreamValuesRequest, ValueUpdate,
};
use tokio::sync::mpsc;
use tonic::transport::Channel;
use tracing::{debug, info};

/// Device information returned from the daemon
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub driver_type: String,
    pub is_movable: bool,
    pub is_readable: bool,
    pub is_triggerable: bool,
    pub is_frame_producer: bool,
}

/// High-level gRPC client for the rust-daq daemon
#[derive(Clone)]
pub struct DaqClient {
    hardware: HardwareServiceClient<Channel>,
}

impl DaqClient {
    /// Connect to the daemon at the given address
    pub async fn connect(address: &str) -> Result<Self> {
        let address = if address.starts_with("http") {
            address.to_string()
        } else {
            format!("http://{}", address)
        };

        info!("Connecting to {}", address);

        let channel = Channel::from_shared(address.clone())?
            .connect()
            .await
            .map_err(|e| anyhow!("Failed to connect to {}: {}", address, e))?;

        info!("Channel established");

        let hardware = HardwareServiceClient::new(channel);

        Ok(Self { hardware })
    }

    /// List all devices from the daemon
    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut client = self.hardware.clone();

        let response = client
            .list_devices(ListDevicesRequest {
                capability_filter: None,
            })
            .await
            .map_err(|e| anyhow!("ListDevices RPC failed: {}", e))?;

        let devices = response
            .into_inner()
            .devices
            .into_iter()
            .map(|d| DeviceInfo {
                id: d.id,
                name: d.name,
                driver_type: d.driver_type,
                is_movable: d.is_movable,
                is_readable: d.is_readable,
                is_triggerable: d.is_triggerable,
                is_frame_producer: d.is_frame_producer,
            })
            .collect();

        Ok(devices)
    }

    /// Move a device to an absolute position
    pub async fn move_absolute(&self, device_id: &str, position: f64) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("MoveAbsolute {} to {}", device_id, position);

        let response = client
            .move_absolute(MoveRequest {
                device_id: device_id.to_string(),
                value: position,
            })
            .await
            .map_err(|e| anyhow!("MoveAbsolute RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("Move failed: {}", resp.error_message));
        }

        Ok(resp.final_position)
    }

    /// Move a device by a relative amount
    pub async fn move_relative(&self, device_id: &str, delta: f64) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("MoveRelative {} by {}", device_id, delta);

        let response = client
            .move_relative(MoveRequest {
                device_id: device_id.to_string(),
                value: delta,
            })
            .await
            .map_err(|e| anyhow!("MoveRelative RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("Move failed: {}", resp.error_message));
        }

        Ok(resp.final_position)
    }

    /// Stop motion on a device
    pub async fn stop_motion(&self, device_id: &str) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("StopMotion {}", device_id);

        let response = client
            .stop_motion(StopMotionRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("StopMotion RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("Stop failed"));
        }

        Ok(resp.stopped_position)
    }

    /// Read a single value from a device
    pub async fn read_value(&self, device_id: &str) -> Result<(f64, String)> {
        let mut client = self.hardware.clone();

        let response = client
            .read_value(ReadValueRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("ReadValue RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("Read failed: {}", resp.error_message));
        }

        Ok((resp.value, resp.units))
    }

    /// Start streaming values from a device
    ///
    /// Returns a receiver channel that yields value updates.
    pub async fn stream_values(
        &self,
        device_id: &str,
        rate_hz: u32,
    ) -> Result<mpsc::Receiver<ValueUpdate>> {
        let mut client = self.hardware.clone();

        let response = client
            .stream_values(StreamValuesRequest {
                device_id: device_id.to_string(),
                rate_hz,
            })
            .await
            .map_err(|e| anyhow!("StreamValues RPC failed: {}", e))?;

        let mut stream = response.into_inner();

        // Create a channel to forward updates
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Ok(Some(update)) = stream.message().await {
                if tx.send(update).await.is_err() {
                    // Receiver dropped, stop streaming
                    break;
                }
            }
            debug!("Value stream ended");
        });

        Ok(rx)
    }
}
