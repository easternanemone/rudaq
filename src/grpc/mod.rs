pub mod hardware_service;
pub mod scan_service;
/// gRPC server for remote DAQ control (Phase 3)
///
/// This module provides a gRPC server that exposes the DAQ system for remote control.
/// It allows clients to upload and execute Rhai scripts, monitor system status,
/// stream measurement data, and directly control hardware devices.
///
/// # Features
/// - Script upload and validation
/// - Remote script execution
/// - Status monitoring
/// - Live data streaming
/// - Direct hardware control (bd-4x6q)
/// - Coordinated multi-axis scanning (bd-4le6)
///
/// # Example
/// ```no_run
/// use rust_daq::grpc::start_server;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let addr = "127.0.0.1:50051".parse()?;
///     start_server(addr).await?;
///     Ok(())
/// }
/// ```
pub mod server;

/// Protocol Buffer definitions for the DAQ Control Service
pub mod proto {
    //! Generated Protocol Buffer definitions from `proto/daq.proto`
    //!
    //! This module contains auto-generated code and provides:
    //! - `ControlService` trait for script management
    //! - `HardwareService` trait for direct device control (bd-4x6q)
    //! - `ScanService` trait for coordinated scans (bd-4le6)
    //! - Server and client implementations for all services
    //! - Request/Response message types for all RPC methods

    tonic::include_proto!("daq");
}

pub use hardware_service::HardwareServiceImpl;
pub use scan_service::ScanServiceImpl;
pub use server::{start_server, start_server_with_hardware, DaqServer};

// Re-export commonly used proto types - ControlService
pub use proto::control_service_client::ControlServiceClient;
pub use proto::control_service_server::{ControlService, ControlServiceServer};
pub use proto::{
    DataPoint, MeasurementRequest, ScriptStatus, StartRequest, StartResponse, StatusRequest,
    StopRequest, StopResponse, SystemStatus, UploadRequest, UploadResponse,
};

// Re-export HardwareService types (bd-4x6q)
pub use proto::hardware_service_client::HardwareServiceClient;
pub use proto::hardware_service_server::{HardwareService, HardwareServiceServer};
pub use proto::{
    DeviceInfo, DeviceMetadata, DeviceStateRequest, DeviceStateResponse, ListDevicesRequest,
    ListDevicesResponse, MoveRequest, MoveResponse, PositionUpdate, ReadValueRequest,
    ReadValueResponse, ValueUpdate,
};

// Re-export ScanService types (bd-4le6)
pub use proto::scan_service_client::ScanServiceClient;
pub use proto::scan_service_server::{ScanService, ScanServiceServer};
pub use proto::{
    AxisConfig, CreateScanRequest, CreateScanResponse, ScanConfig, ScanProgress, ScanState,
    ScanStatus, ScanType,
};
