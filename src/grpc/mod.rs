/// gRPC server for remote DAQ control (Phase 3)
///
/// This module provides a gRPC server that exposes the DAQ system for remote control.
/// It allows clients to upload and execute Rhai scripts, monitor system status,
/// and stream measurement data.
///
/// # Features
/// - Script upload and validation
/// - Remote script execution
/// - Status monitoring
/// - Live data streaming
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
    //! - `ControlService` trait for server implementation
    //! - `ControlServiceServer` for serving the gRPC API
    //! - `ControlServiceClient` for remote control clients
    //! - Request/Response message types for all RPC methods

    tonic::include_proto!("daq");
}

pub use server::{DaqServer, start_server};

// Re-export commonly used proto types
pub use proto::control_service_server::{ControlService, ControlServiceServer};
pub use proto::control_service_client::ControlServiceClient;
pub use proto::{
    DataPoint, MeasurementRequest, ScriptStatus, StartRequest, StartResponse,
    StatusRequest, StopRequest, StopResponse, SystemStatus, UploadRequest, UploadResponse,
};
