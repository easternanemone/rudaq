//! gRPC client for communicating with the DAQ daemon.

use std::time::Duration;

use anyhow::Result;
use tonic::transport::Channel;

use crate::connection::DaemonAddress;

/// gRPC channel configuration for connection reliability.
///
/// These settings are tuned for a DAQ GUI that maintains a persistent connection
/// to a local or networked daemon, with emphasis on fast failure detection.
pub struct ChannelConfig {
    /// Connection timeout (how long to wait for initial connection)
    pub connect_timeout: Duration,
    /// Request timeout (default timeout for individual RPC calls)
    pub request_timeout: Duration,
    /// HTTP/2 keepalive interval (how often to send keepalive pings)
    pub keepalive_interval: Duration,
    /// Keepalive timeout (how long to wait for keepalive response)
    pub keepalive_timeout: Duration,
    /// Whether to send keepalive pings even when idle
    pub keepalive_while_idle: bool,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            keepalive_interval: Duration::from_secs(10),
            keepalive_timeout: Duration::from_secs(60),
            keepalive_while_idle: true,
        }
    }
}

impl ChannelConfig {
    /// Fast configuration for local connections (localhost/Tailscale).
    #[must_use]
    #[allow(dead_code)]
    pub fn fast() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            keepalive_interval: Duration::from_secs(15),
            keepalive_timeout: Duration::from_secs(5),
            keepalive_while_idle: true,
        }
    }
}
use daq_proto::daq::{
    control_service_client::ControlServiceClient,
    hardware_service_client::HardwareServiceClient,
    module_service_client::ModuleServiceClient,
    run_engine_service_client::RunEngineServiceClient,
    scan_service_client::ScanServiceClient,
    storage_service_client::StorageServiceClient,
    AssignDeviceRequest,
    CreateModuleRequest,
    // Scan types
    CreateScanRequest,
    // Request/Response types
    DaemonInfoRequest,
    DeviceCommandRequest,
    DeviceStateRequest,
    FrameData,
    GetParameterRequest,
    GetRecordingStatusRequest,
    // Storage types
    GetStorageConfigRequest,
    ListAcquisitionsRequest,
    ListDevicesRequest,
    ListExecutionsRequest,
    // Module types
    ListModuleTypesRequest,
    ListModulesRequest,
    // Parameter types (bd-cdh5.1)
    ListParametersRequest,
    ListScansRequest,
    ListScriptsRequest,
    MoveRequest,
    ObservableValue,
    PauseScanRequest,
    QueuePlanRequest,
    QueuePlanResponse,
    ReadValueRequest,
    ResumeScanRequest,
    ScanConfig,
    SetParameterRequest,
    StartModuleRequest,
    StartRecordingRequest,
    // Script execution types (Phase 6: bd-uu9t)
    StartRequest as ScriptStartRequest,
    StartResponse as ScriptStartResponse,
    StartScanRequest,
    // Camera streaming with quality control
    StartStreamRequest,
    StopModuleRequest,
    StopRecordingRequest,
    StopRequest as ScriptStopRequest,
    StopResponse as ScriptStopResponse,
    StopScanRequest,
    StopStreamRequest,
    StreamDocumentsRequest,
    StreamFramesRequest,
    // Observable streaming (bd-qqjq stub for bd-r5vb)
    StreamObservablesRequest,
    StreamQuality,
    UploadRequest as ScriptUploadRequest,
    UploadResponse as ScriptUploadResponse,
};

/// gRPC client wrapper for the DAQ daemon
#[derive(Clone)]
pub struct DaqClient {
    control: ControlServiceClient<Channel>,
    hardware: HardwareServiceClient<Channel>,
    /// Dedicated client for streaming RPCs (no request timeout)
    hardware_streaming: HardwareServiceClient<Channel>,
    scan: ScanServiceClient<Channel>,
    storage: StorageServiceClient<Channel>,
    module: ModuleServiceClient<Channel>,
    run_engine: RunEngineServiceClient<Channel>,
}

/// Maximum message size for gRPC (64 MB for high-resolution camera frames)
/// Must match server's max_encoding_message_size in server.rs
const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

impl DaqClient {
    /// Connect to the DAQ daemon at the given address with default configuration.
    ///
    /// The address is validated and normalized by `DaemonAddress` before connection.
    /// TLS is automatically enabled for `https://` addresses.
    pub async fn connect(address: &DaemonAddress) -> Result<Self> {
        Self::connect_with_config(address, ChannelConfig::default()).await
    }

    /// Connect with custom channel configuration.
    ///
    /// Use `ChannelConfig::fast()` for local/Tailscale connections with quicker
    /// failure detection, or customize timeouts for high-latency networks.
    pub async fn connect_with_config(
        address: &DaemonAddress,
        config: ChannelConfig,
    ) -> Result<Self> {
        // Channel with request timeout for regular RPCs
        let endpoint = Channel::from_shared(address.as_str().to_string())?
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .http2_keep_alive_interval(config.keepalive_interval)
            .keep_alive_timeout(config.keepalive_timeout)
            .keep_alive_while_idle(config.keepalive_while_idle);

        // Streaming channel WITHOUT request timeout for long-lived streaming RPCs
        // This prevents the 30-second default timeout from cancelling frame streams
        // TCP tuning for reduced frame drops during streaming:
        // - tcp_nodelay: Disables Nagle's algorithm to reduce latency
        // - buffer_size: Larger buffer to absorb network jitter (1MB)
        // - initial_stream_window_size: Larger window for high-bandwidth frame streams
        let streaming_endpoint = Channel::from_shared(address.as_str().to_string())?
            .connect_timeout(config.connect_timeout)
            // No .timeout() call - streaming RPCs should not have a request timeout
            .http2_keep_alive_interval(config.keepalive_interval)
            .keep_alive_timeout(config.keepalive_timeout)
            .keep_alive_while_idle(config.keepalive_while_idle)
            .tcp_nodelay(true) // Disable Nagle's algorithm for lower latency
            .buffer_size(1024 * 1024) // 1MB buffer for high-bandwidth streaming
            .initial_stream_window_size(1024 * 1024); // 1MB initial window

        // TODO(bd-otbx): Add TLS configuration for https:// addresses
        // if address.is_tls() {
        //     let tls_config = load_tls_config()?;
        //     endpoint = endpoint.tls_config(tls_config)?;
        // }

        let channel = endpoint.connect().await?;
        let streaming_channel = streaming_endpoint.connect().await?;

        Ok(Self {
            control: ControlServiceClient::new(channel.clone()),
            // Hardware client needs larger message size for camera frame streaming
            hardware: HardwareServiceClient::new(channel.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            // Dedicated streaming client without request timeout
            hardware_streaming: HardwareServiceClient::new(streaming_channel)
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            scan: ScanServiceClient::new(channel.clone()),
            storage: StorageServiceClient::new(channel.clone()),
            module: ModuleServiceClient::new(channel.clone()),
            run_engine: RunEngineServiceClient::new(channel),
        })
    }

    /// Perform a lightweight health check by calling GetDaemonInfo.
    ///
    /// Returns `Ok(())` if the daemon is responsive, `Err` otherwise.
    /// This is used by the ConnectionManager to detect stale connections.
    pub async fn health_check(&mut self) -> Result<()> {
        self.control
            .get_daemon_info(daq_proto::daq::DaemonInfoRequest {})
            .await?;
        Ok(())
    }

    /// Get daemon information (version, capabilities, etc.)
    pub async fn get_daemon_info(&mut self) -> Result<daq_proto::daq::DaemonInfoResponse> {
        let response = self.control.get_daemon_info(DaemonInfoRequest {}).await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Hardware Service
    // =========================================================================

    /// List all devices
    pub async fn list_devices(&mut self) -> Result<Vec<daq_proto::daq::DeviceInfo>> {
        let response = self
            .hardware
            .list_devices(ListDevicesRequest {
                capability_filter: None,
            })
            .await?;
        Ok(response.into_inner().devices)
    }

    /// Get device state
    pub async fn get_device_state(
        &mut self,
        device_id: &str,
    ) -> Result<daq_proto::daq::DeviceStateResponse> {
        let response = self
            .hardware
            .get_device_state(DeviceStateRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Move device to absolute position
    pub async fn move_absolute(
        &mut self,
        device_id: &str,
        position: f64,
    ) -> Result<daq_proto::daq::MoveResponse> {
        let response = self
            .hardware
            .move_absolute(MoveRequest {
                device_id: device_id.to_string(),
                value: position,
                wait_for_completion: Some(false),
                timeout_ms: None,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Move device by relative amount
    pub async fn move_relative(
        &mut self,
        device_id: &str,
        distance: f64,
    ) -> Result<daq_proto::daq::MoveResponse> {
        let response = self
            .hardware
            .move_relative(MoveRequest {
                device_id: device_id.to_string(),
                value: distance,
                wait_for_completion: Some(false),
                timeout_ms: None,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Read value from device
    pub async fn read_value(
        &mut self,
        device_id: &str,
    ) -> Result<daq_proto::daq::ReadValueResponse> {
        let response = self
            .hardware
            .read_value(ReadValueRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Control Service (Scripts)
    // =========================================================================

    /// List all scripts
    pub async fn list_scripts(&mut self) -> Result<Vec<daq_proto::daq::ScriptInfo>> {
        let response = self.control.list_scripts(ListScriptsRequest {}).await?;
        Ok(response.into_inner().scripts)
    }

    /// List all executions
    pub async fn list_executions(&mut self) -> Result<Vec<daq_proto::daq::ScriptStatus>> {
        let response = self
            .control
            .list_executions(ListExecutionsRequest {
                script_id: None,
                state: None,
            })
            .await?;
        Ok(response.into_inner().executions)
    }

    /// Upload a script to the daemon (Phase 6: bd-uu9t)
    #[allow(dead_code)]
    pub async fn upload_script(
        &mut self,
        name: &str,
        content: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<ScriptUploadResponse> {
        let response = self
            .control
            .upload_script(ScriptUploadRequest {
                name: name.to_string(),
                script_content: content.to_string(),
                metadata,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Start execution of an uploaded script (Phase 6: bd-uu9t)
    pub async fn start_script(
        &mut self,
        script_id: &str,
        parameters: std::collections::HashMap<String, String>,
    ) -> Result<ScriptStartResponse> {
        let response = self
            .control
            .start_script(ScriptStartRequest {
                script_id: script_id.to_string(),
                parameters,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stop a running script execution (Phase 6: bd-uu9t)
    pub async fn stop_script(
        &mut self,
        execution_id: &str,
        force: bool,
    ) -> Result<ScriptStopResponse> {
        let response = self
            .control
            .stop_script(ScriptStopRequest {
                execution_id: execution_id.to_string(),
                force,
            })
            .await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Scan Service
    // =========================================================================

    /// List all scans
    pub async fn list_scans(&mut self) -> Result<Vec<daq_proto::daq::ScanStatus>> {
        let response = self
            .scan
            .list_scans(ListScansRequest { state_filter: None })
            .await?;
        Ok(response.into_inner().scans)
    }

    /// Create a new scan
    pub async fn create_scan(
        &mut self,
        config: ScanConfig,
    ) -> Result<daq_proto::daq::CreateScanResponse> {
        let response = self
            .scan
            .create_scan(CreateScanRequest {
                config: Some(config),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Start a scan
    pub async fn start_scan(&mut self, scan_id: &str) -> Result<daq_proto::daq::StartScanResponse> {
        let response = self
            .scan
            .start_scan(StartScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Pause a scan
    pub async fn pause_scan(&mut self, scan_id: &str) -> Result<daq_proto::daq::PauseScanResponse> {
        let response = self
            .scan
            .pause_scan(PauseScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Resume a scan
    pub async fn resume_scan(
        &mut self,
        scan_id: &str,
    ) -> Result<daq_proto::daq::ResumeScanResponse> {
        let response = self
            .scan
            .resume_scan(ResumeScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stop a scan
    pub async fn stop_scan(
        &mut self,
        scan_id: &str,
        emergency: bool,
    ) -> Result<daq_proto::daq::StopScanResponse> {
        let response = self
            .scan
            .stop_scan(StopScanRequest {
                scan_id: scan_id.to_string(),
                emergency_stop: emergency,
            })
            .await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Storage Service
    // =========================================================================

    /// Get storage configuration
    pub async fn get_storage_config(&mut self) -> Result<daq_proto::daq::StorageConfig> {
        let response = self
            .storage
            .get_storage_config(GetStorageConfigRequest {})
            .await?;
        Ok(response.into_inner())
    }

    /// Get recording status
    pub async fn get_recording_status(&mut self) -> Result<daq_proto::daq::RecordingStatus> {
        let response = self
            .storage
            .get_recording_status(GetRecordingStatusRequest { recording_id: None })
            .await?;
        Ok(response.into_inner())
    }

    /// Start recording
    pub async fn start_recording(
        &mut self,
        name: &str,
    ) -> Result<daq_proto::daq::StartRecordingResponse> {
        let response = self
            .storage
            .start_recording(StartRecordingRequest {
                name: name.to_string(),
                metadata: Default::default(),
                config_override: None,
                scan_id: None,
                run_uid: None,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stop recording
    pub async fn stop_recording(&mut self) -> Result<daq_proto::daq::StopRecordingResponse> {
        let response = self
            .storage
            .stop_recording(StopRecordingRequest {
                recording_id: None,
                final_metadata: Default::default(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// List acquisitions
    pub async fn list_acquisitions(&mut self) -> Result<Vec<daq_proto::daq::AcquisitionSummary>> {
        let response = self
            .storage
            .list_acquisitions(ListAcquisitionsRequest {
                name_pattern: None,
                after_timestamp_ns: None,
                before_timestamp_ns: None,
                limit: Some(100),
                offset: None,
            })
            .await?;
        Ok(response.into_inner().acquisitions)
    }

    // =========================================================================
    // Module Service
    // =========================================================================

    /// List module types
    pub async fn list_module_types(&mut self) -> Result<Vec<daq_proto::daq::ModuleTypeSummary>> {
        let response = self
            .module
            .list_module_types(ListModuleTypesRequest {
                required_capability: None,
            })
            .await?;
        Ok(response.into_inner().module_types)
    }

    /// List module instances
    pub async fn list_modules(&mut self) -> Result<Vec<daq_proto::daq::ModuleStatus>> {
        let response = self
            .module
            .list_modules(ListModulesRequest {
                type_filter: None,
                state_filter: None,
            })
            .await?;
        Ok(response.into_inner().modules)
    }

    /// Create a module instance
    pub async fn create_module(
        &mut self,
        type_id: &str,
        name: &str,
    ) -> Result<daq_proto::daq::CreateModuleResponse> {
        let response = self
            .module
            .create_module(CreateModuleRequest {
                type_id: type_id.to_string(),
                instance_name: name.to_string(),
                initial_config: Default::default(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Start a module
    pub async fn start_module(
        &mut self,
        module_id: &str,
    ) -> Result<daq_proto::daq::StartModuleResponse> {
        let response = self
            .module
            .start_module(StartModuleRequest {
                module_id: module_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stop a module
    pub async fn stop_module(
        &mut self,
        module_id: &str,
    ) -> Result<daq_proto::daq::StopModuleResponse> {
        let response = self
            .module
            .stop_module(StopModuleRequest {
                module_id: module_id.to_string(),
                force: false,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Assign device to module role
    #[allow(dead_code)]
    pub async fn assign_device(
        &mut self,
        module_id: &str,
        role_id: &str,
        device_id: &str,
    ) -> Result<daq_proto::daq::AssignDeviceResponse> {
        let response = self
            .module
            .assign_device(AssignDeviceRequest {
                module_id: module_id.to_string(),
                role_id: role_id.to_string(),
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Start camera stream (frames logged to Rerun)
    /// If frame_count is None, streams indefinitely until stopped.
    pub async fn start_stream(
        &mut self,
        device_id: &str,
        frame_count: Option<u32>,
    ) -> Result<daq_proto::daq::StartStreamResponse> {
        let response = self
            .hardware
            .start_stream(StartStreamRequest {
                device_id: device_id.to_string(),
                frame_count,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stop camera stream
    pub async fn stop_stream(
        &mut self,
        device_id: &str,
    ) -> Result<daq_proto::daq::StopStreamResponse> {
        let response = self
            .hardware
            .stop_stream(StopStreamRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stream frames from a camera device
    ///
    /// Returns a gRPC stream of FrameData. The max_fps parameter limits the
    /// frame rate to prevent overwhelming the GUI (0 = no limit).
    ///
    /// The quality parameter controls server-side downsampling:
    /// - `Full`: Full resolution (default)
    /// - `Preview`: 2x2 binned (~4x smaller)
    /// - `Fast`: 4x4 binned (~16x smaller)
    ///
    /// Uses a dedicated streaming channel without request timeout to prevent
    /// long-lived streams from being cancelled by the default 30-second timeout.
    pub async fn stream_frames(
        &mut self,
        device_id: &str,
        max_fps: u32,
        quality: StreamQuality,
    ) -> Result<impl futures::Stream<Item = Result<FrameData, tonic::Status>>> {
        let request = StreamFramesRequest {
            device_id: device_id.to_string(),
            max_fps,
            quality: quality.into(),
        };
        // Use hardware_streaming client (no request timeout) for long-lived streams
        let response = self.hardware_streaming.stream_frames(request).await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Parameter Service (bd-cdh5.1)
    // =========================================================================

    /// List all parameters for a device
    pub async fn list_parameters(
        &mut self,
        device_id: &str,
    ) -> Result<Vec<daq_proto::daq::ParameterDescriptor>> {
        let response = self
            .hardware
            .list_parameters(ListParametersRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner().parameters)
    }

    /// Get a single parameter value
    pub async fn get_parameter(
        &mut self,
        device_id: &str,
        name: &str,
    ) -> Result<daq_proto::daq::ParameterValue> {
        let response = self
            .hardware
            .get_parameter(GetParameterRequest {
                device_id: device_id.to_string(),
                parameter_name: name.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Set a parameter value
    pub async fn set_parameter(
        &mut self,
        device_id: &str,
        name: &str,
        value: &str,
    ) -> Result<daq_proto::daq::SetParameterResponse> {
        let response = self
            .hardware
            .set_parameter(SetParameterRequest {
                device_id: device_id.to_string(),
                parameter_name: name.to_string(),
                value: value.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Execute a specialized device command
    pub async fn execute_device_command(
        &mut self,
        device_id: &str,
        command: &str,
        args: &str,
    ) -> Result<daq_proto::daq::DeviceCommandResponse> {
        let response = self
            .hardware
            .execute_device_command(DeviceCommandRequest {
                device_id: device_id.to_string(),
                command: command.to_string(),
                args: args.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Observable Streaming (bd-qqjq stub for bd-r5vb)
    // =========================================================================

    /// Stream observable values from devices
    ///
    /// Returns a stream of ObservableValue messages from the specified devices
    /// and observables. The server may downsample to the requested rate.
    ///
    /// Uses a dedicated streaming channel without request timeout to prevent
    /// long-lived streams from being cancelled by the default 30-second timeout.
    ///
    /// # Arguments
    ///
    /// * `device_ids` - Device IDs to stream from
    /// * `observable_names` - Observable names to stream (e.g., "power_mw")
    /// * `sample_rate_hz` - Desired sample rate (server may downsample)
    pub async fn stream_observables(
        &mut self,
        device_ids: Vec<String>,
        observable_names: Vec<String>,
        sample_rate_hz: u32,
    ) -> Result<impl futures::Stream<Item = Result<ObservableValue, tonic::Status>>> {
        let request = StreamObservablesRequest {
            device_ids,
            observable_names,
            sample_rate_hz,
        };
        // Use hardware_streaming client (no request timeout) for long-lived streams
        let response = self.hardware_streaming.stream_observables(request).await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // RunEngine Service
    // =========================================================================

    /// Queue a plan for execution
    pub async fn queue_plan(
        &mut self,
        plan_type: &str,
        parameters: std::collections::HashMap<String, String>,
        device_mapping: std::collections::HashMap<String, String>,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<QueuePlanResponse> {
        let response = self
            .run_engine
            .queue_plan(QueuePlanRequest {
                plan_type: plan_type.to_string(),
                parameters,
                device_mapping,
                metadata,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stream documents from plan execution
    pub async fn stream_documents(
        &mut self,
        run_uid: Option<String>,
        doc_types: Vec<i32>,
    ) -> Result<impl futures::Stream<Item = Result<daq_proto::daq::Document, tonic::Status>>> {
        let response = self
            .run_engine
            .stream_documents(StreamDocumentsRequest { run_uid, doc_types })
            .await?;
        Ok(response.into_inner())
    }
}
