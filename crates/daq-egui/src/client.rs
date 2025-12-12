//! gRPC client for communicating with the DAQ daemon.

use anyhow::Result;
use tonic::transport::Channel;

use daq_proto::daq::{
    control_service_client::ControlServiceClient,
    hardware_service_client::HardwareServiceClient,
    scan_service_client::ScanServiceClient,
    // Request/Response types
    DaemonInfoRequest, ListDevicesRequest, ListScansRequest, ListScriptsRequest,
    ListExecutionsRequest, MoveRequest, ReadValueRequest, DeviceStateRequest,
};

/// gRPC client wrapper for the DAQ daemon
#[derive(Clone)]
pub struct DaqClient {
    control: ControlServiceClient<Channel>,
    hardware: HardwareServiceClient<Channel>,
    scan: ScanServiceClient<Channel>,
}

impl DaqClient {
    /// Connect to the DAQ daemon at the given address
    pub async fn connect(address: &str) -> Result<Self> {
        let channel = Channel::from_shared(address.to_string())?
            .connect()
            .await?;

        Ok(Self {
            control: ControlServiceClient::new(channel.clone()),
            hardware: HardwareServiceClient::new(channel.clone()),
            scan: ScanServiceClient::new(channel),
        })
    }

    /// Get daemon information (version, capabilities, etc.)
    pub async fn get_daemon_info(&mut self) -> Result<daq_proto::daq::DaemonInfoResponse> {
        let response = self.control.get_daemon_info(DaemonInfoRequest {}).await?;
        Ok(response.into_inner())
    }

    /// List all devices
    pub async fn list_devices(&mut self) -> Result<Vec<daq_proto::daq::DeviceInfo>> {
        let response = self.hardware.list_devices(ListDevicesRequest {
            capability_filter: None,
        }).await?;
        Ok(response.into_inner().devices)
    }

    /// Get device state
    pub async fn get_device_state(&mut self, device_id: &str) -> Result<daq_proto::daq::DeviceStateResponse> {
        let response = self.hardware.get_device_state(DeviceStateRequest {
            device_id: device_id.to_string(),
        }).await?;
        Ok(response.into_inner())
    }

    /// Move device to absolute position
    pub async fn move_absolute(&mut self, device_id: &str, position: f64) -> Result<daq_proto::daq::MoveResponse> {
        let response = self.hardware.move_absolute(MoveRequest {
            device_id: device_id.to_string(),
            value: position,
            wait_for_completion: Some(false),
            timeout_ms: None,
        }).await?;
        Ok(response.into_inner())
    }

    /// Move device by relative amount
    pub async fn move_relative(&mut self, device_id: &str, distance: f64) -> Result<daq_proto::daq::MoveResponse> {
        let response = self.hardware.move_relative(MoveRequest {
            device_id: device_id.to_string(),
            value: distance,
            wait_for_completion: Some(false),
            timeout_ms: None,
        }).await?;
        Ok(response.into_inner())
    }

    /// Read value from device
    pub async fn read_value(&mut self, device_id: &str) -> Result<daq_proto::daq::ReadValueResponse> {
        let response = self.hardware.read_value(ReadValueRequest {
            device_id: device_id.to_string(),
        }).await?;
        Ok(response.into_inner())
    }

    /// List all scripts
    pub async fn list_scripts(&mut self) -> Result<Vec<daq_proto::daq::ScriptInfo>> {
        let response = self.control.list_scripts(ListScriptsRequest {}).await?;
        Ok(response.into_inner().scripts)
    }

    /// List all executions
    pub async fn list_executions(&mut self) -> Result<Vec<daq_proto::daq::ScriptStatus>> {
        let response = self.control.list_executions(ListExecutionsRequest {
            script_id: None,
            state: None,
        }).await?;
        Ok(response.into_inner().executions)
    }

    /// List all scans
    pub async fn list_scans(&mut self) -> Result<Vec<daq_proto::daq::ScanStatus>> {
        let response = self.scan.list_scans(ListScansRequest {
            state_filter: None,
        }).await?;
        Ok(response.into_inner().scans)
    }
}
