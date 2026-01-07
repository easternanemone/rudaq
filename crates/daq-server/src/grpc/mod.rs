pub mod custom_health_service;
pub mod error_mapping;
#[cfg(test)]
mod error_mapping_tests;
pub mod hardware_service;
pub mod health_service;
#[cfg(feature = "metrics")]
pub mod metrics_service;
pub mod module_service;
pub mod plugin_service;
pub mod preset_service;
pub mod run_engine_service;
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
/// use daq_server::grpc::start_server;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let addr = "127.0.0.1:50051".parse()?;
///     start_server(addr).await?;
///     Ok(())
/// }
/// ```
#[cfg(feature = "server")]
pub mod server;
pub mod storage_service;

/// Protocol Buffer definitions for the DAQ Control Service
///
/// Re-exported from daq-proto crate to maintain API compatibility.
#[allow(missing_docs)]
pub mod proto {
    //! Generated Protocol Buffer definitions from `proto/daq.proto`
    //!
    //! This module contains auto-generated code and provides:
    //! - `ControlService` trait for script management
    //! - `HardwareService` trait for direct device control (bd-4x6q)
    //! - `ScanService` trait for coordinated scans (bd-4le6)
    //! - Server and client implementations for all services
    //! - Request/Response message types for all RPC methods
    //!
    //! Note: These types are generated in the daq-proto crate and
    //! re-exported here for backwards compatibility.

    pub use daq_proto::daq::*;

    pub mod health {
        pub use daq_proto::health::*;
    }
}

pub use hardware_service::HardwareServiceImpl;
pub use health_service::HealthServiceImpl;
#[cfg(feature = "metrics")]
pub use metrics_service::{DaqMetrics, MetricsServerHandle, start_metrics_server};
pub use module_service::ModuleServiceImpl;
pub use plugin_service::PluginServiceImpl;
pub use preset_service::{PresetServiceImpl, default_preset_storage_path};
pub use run_engine_service::RunEngineServiceImpl;
pub use scan_service::ScanServiceImpl;
#[cfg(feature = "server")]
pub use server::{DaqServer, start_server, start_server_with_hardware};
pub use storage_service::StorageServiceImpl;

// Error mapping (bd-cxvg)
pub use error_mapping::{map_daq_error_to_status, DaqResultExt};

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
    DeviceInfo,
    DeviceMetadata,
    DeviceStateRequest,
    DeviceStateResponse,
    DeviceStateSubscribeRequest,
    DeviceStateUpdate,
    GetEmissionRequest,
    GetEmissionResponse,
    GetExposureRequest,
    GetExposureResponse,
    GetParameterRequest,
    GetShutterRequest,
    GetShutterResponse,
    GetWavelengthRequest,
    GetWavelengthResponse,
    ListDevicesRequest,
    ListDevicesResponse,
    // Parameter listing (bd-ibcn.3)
    ListParametersRequest,
    ListParametersResponse,
    MoveRequest,
    MoveResponse,
    // Parameter descriptor for dynamic control panels (bd-ibcn.3)
    ParameterDescriptor,
    ParameterValue,
    PositionUpdate,
    ReadValueRequest,
    ReadValueResponse,
    SetEmissionRequest,
    SetEmissionResponse,
    // Exposure control types (bd-tm0b)
    SetExposureRequest,
    SetExposureResponse,
    // Parameter control types (bd-lxwp)
    SetParameterRequest,
    SetParameterResponse,
    // Laser control types (bd-pwjo)
    SetShutterRequest,
    SetShutterResponse,
    SetWavelengthRequest,
    SetWavelengthResponse,
    // Frame streaming types (bd-p6vz)
    StartStreamRequest,
    StartStreamResponse,
    StopMotionRequest,
    StopMotionResponse,
    StopStreamRequest,
    StopStreamResponse,
    StreamValuesRequest,
    ValueUpdate,
};

// Re-export ScanService types (bd-4le6)
pub use proto::scan_service_client::ScanServiceClient;
pub use proto::scan_service_server::{ScanService, ScanServiceServer};
pub use proto::{
    AxisConfig, CreateScanRequest, CreateScanResponse, GetScanStatusRequest, ListScansRequest,
    ListScansResponse, PauseScanRequest, PauseScanResponse, ResumeScanRequest, ResumeScanResponse,
    ScanConfig, ScanProgress, ScanState, ScanStatus, ScanType, StartScanRequest, StartScanResponse,
    StopScanRequest, StopScanResponse, StreamScanProgressRequest,
};

// Re-export PresetService types (bd-akcm)
pub use proto::preset_service_client::PresetServiceClient;
pub use proto::preset_service_server::{PresetService, PresetServiceServer};
pub use proto::{
    DeletePresetRequest, DeletePresetResponse, GetPresetRequest, ListPresetsRequest,
    ListPresetsResponse, LoadPresetRequest, LoadPresetResponse, Preset, PresetMetadata,
    SavePresetRequest, SavePresetResponse,
};

// Re-export ModuleService types (bd-xx7f)
pub use proto::module_service_client::ModuleServiceClient;
pub use proto::module_service_server::{ModuleService, ModuleServiceServer};
pub use proto::{
    // Device assignment
    AssignDeviceRequest,
    AssignDeviceResponse,
    // Module configuration
    ConfigureModuleRequest,
    ConfigureModuleResponse,
    // Module lifecycle
    CreateModuleRequest,
    CreateModuleResponse,
    DeleteModuleRequest,
    DeleteModuleResponse,
    DeviceAssignment,
    GetModuleConfigRequest,
    GetModuleStatusRequest,
    GetModuleTypeInfoRequest,
    ListAssignmentsRequest,
    ListAssignmentsResponse,
    // Module type discovery
    ListModuleTypesRequest,
    ListModuleTypesResponse,
    ListModulesRequest,
    ListModulesResponse,
    ModuleConfig,
    ModuleDataPoint,
    ModuleEvent,
    ModuleEventSeverity,
    ModuleParameter,
    ModuleRole,
    ModuleState,
    ModuleStatus,
    ModuleTypeInfo,
    ModuleTypeSummary,
    PauseModuleRequest,
    PauseModuleResponse,
    ResumeModuleRequest,
    ResumeModuleResponse,
    // Module execution control
    StartModuleRequest,
    StartModuleResponse,
    StopModuleRequest,
    StopModuleResponse,
    StreamModuleDataRequest,
    // Module event streaming
    StreamModuleEventsRequest,
    UnassignDeviceRequest,
    UnassignDeviceResponse,
};

// Re-export RunEngineService types (bd-niy4)
pub use proto::run_engine_service_client::RunEngineServiceClient;
pub use proto::run_engine_service_server::{RunEngineService, RunEngineServiceServer};
pub use proto::{
    AbortPlanRequest,
    AbortPlanResponse,
    DescriptorDocument,
    Document,
    DocumentType,
    EngineState,
    EngineStatus,
    EventDocument,
    GetEngineStatusRequest,
    GetPlanTypeInfoRequest,
    HaltEngineRequest,
    HaltEngineResponse,
    // Plan type discovery
    ListPlanTypesRequest,
    ListPlanTypesResponse,
    PauseEngineRequest,
    PauseEngineResponse,
    PlanDeviceRole,
    PlanParameter,
    PlanTypeInfo,
    PlanTypeSummary,
    // Plan execution
    QueuePlanRequest,
    QueuePlanResponse,
    ResumeEngineRequest,
    ResumeEngineResponse,
    StartDocument,
    StartEngineRequest,
    StartEngineResponse,
    StopDocument,
    // Document streaming
    StreamDocumentsRequest,
};

// Re-export StorageService types (bd-p6im)
pub use proto::storage_service_client::StorageServiceClient;
pub use proto::storage_service_server::{StorageService, StorageServiceServer};
pub use proto::{
    AcquisitionInfo,
    AcquisitionSummary,
    // Storage configuration
    ConfigureStorageRequest,
    ConfigureStorageResponse,
    DatasetInfo,
    DeleteAcquisitionRequest,
    DeleteAcquisitionResponse,
    // Data export
    FlushToStorageRequest,
    FlushToStorageResponse,
    GetAcquisitionInfoRequest,
    GetRecordingStatusRequest,
    GetStorageConfigRequest,
    Hdf5Config,
    Hdf5Structure,
    // Acquisition management
    ListAcquisitionsRequest,
    ListAcquisitionsResponse,
    RecordingProgress,
    RecordingState,
    RecordingStatus,
    // Recording control
    StartRecordingRequest,
    StartRecordingResponse,
    StopRecordingRequest,
    StopRecordingResponse,
    StorageConfig,
    StreamRecordingProgressRequest,
};

// Re-export PluginService types (bd-22si.6.1)
pub use proto::plugin_service_client::PluginServiceClient;
pub use proto::plugin_service_server::{PluginService, PluginServiceServer};
pub use proto::{
    DestroyPluginInstanceRequest,
    DestroyPluginInstanceResponse,
    GetPluginInfoRequest,
    GetPluginInstanceStatusRequest,
    ListPluginInstancesRequest,
    ListPluginInstancesResponse,
    // Plugin discovery
    ListPluginsRequest,
    ListPluginsResponse,
    PluginActionable,
    PluginAxis,
    PluginCapabilities,
    PluginInfo,
    PluginInstanceStatus,
    PluginInstanceSummary,
    PluginLoggable,
    PluginMovable,
    PluginProtocol,
    PluginReadable,
    PluginScriptable,
    PluginSettable,
    PluginSummary,
    PluginSwitchable,
    PluginUiElement,
    // Plugin instance management
    SpawnPluginRequest,
    SpawnPluginResponse,
};
