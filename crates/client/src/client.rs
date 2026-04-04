//! gRPC client for communicating with the DAQ daemon.
//!
//! On native platforms, uses `tonic::transport::Channel` (HTTP/2).
//! On WASM, uses `tonic-web-wasm-client` (gRPC-web over HTTP/1.1).

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use anyhow::Result;

#[cfg(not(target_arch = "wasm32"))]
use tonic::transport::Channel;

#[cfg(not(target_arch = "wasm32"))]
use crate::connection::DaemonAddress;

/// Platform-specific gRPC transport type.
///
/// - Native: `tonic::transport::Channel` (HTTP/2 with keepalive, TLS, etc.)
/// - WASM: `tonic_web_wasm_client::Client` (gRPC-web over HTTP/1.1 via browser fetch)
#[cfg(not(target_arch = "wasm32"))]
pub type Transport = Channel;

#[cfg(target_arch = "wasm32")]
pub type Transport = tonic_web_wasm_client::Client;

/// gRPC channel configuration for connection reliability (native only).
///
/// These settings are tuned for a DAQ GUI that maintains a persistent connection
/// to a local or networked daemon, with emphasis on fast failure detection.
/// Not applicable to WASM (browser manages HTTP connection lifecycle).
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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
use protocol::daq::{
    // RunEngine control types
    AbortPlanRequest,
    AbortPlanResponse,
    AssignDeviceRequest,
    CalibrationSnapshot,
    ClearCalibrationSnapshotRequest,
    ClearCalibrationSnapshotResponse,
    CreateModuleRequest,
    // Scan types
    CreateScanRequest,
    // Request/Response types
    DaemonInfoRequest,
    DeviceCommandRequest,
    DeviceStateRequest,
    EngineStatus,
    FrameData,
    // Laser control types (bd-pwjo)
    GetEmissionRequest,
    GetEngineStatusRequest,
    GetParameterRequest,
    GetRecordingStatusRequest,
    GetShutterRequest,
    // Storage types
    GetStorageConfigRequest,
    GetWavelengthRequest,
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
    PauseEngineRequest,
    PauseEngineResponse,
    PauseScanRequest,
    QueuePlanRequest,
    QueuePlanResponse,
    ReadValueRequest,
    ResumeEngineRequest,
    ResumeEngineResponse,
    ResumeScanRequest,
    ScanConfig,
    SetCalibrationSnapshotRequest,
    SetCalibrationSnapshotResponse,
    SetEmissionRequest,
    SetParameterRequest,
    SetShutterRequest,
    SetWavelengthRequest,
    SpectrumPayload,
    SpectrumStreamRequest,
    StartEngineRequest,
    StartEngineResponse,
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
    StreamEngineStatusRequest,
    StreamFramesRequest,
    // Observable streaming (bd-qqjq stub for bd-r5vb)
    StreamObservablesRequest,
    StreamQuality,
    UploadRequest as ScriptUploadRequest,
    UploadResponse as ScriptUploadResponse,
    control_service_client::ControlServiceClient,
    hardware_service_client::HardwareServiceClient,
    module_service_client::ModuleServiceClient,
    run_engine_service_client::RunEngineServiceClient,
    scan_service_client::ScanServiceClient,
    storage_service_client::StorageServiceClient,
};
use protocol::ni_daq::ni_daq_service_client::NiDaqServiceClient;

/// gRPC client wrapper for the DAQ daemon.
///
/// Uses platform-specific transport: native HTTP/2 `Channel` or WASM gRPC-web `Client`.
#[derive(Clone)]
pub struct DaqClient {
    control: ControlServiceClient<Transport>,
    /// Dedicated ControlService client for long-lived streaming RPCs (no request timeout)
    control_streaming: ControlServiceClient<Transport>,
    hardware: HardwareServiceClient<Transport>,
    /// Dedicated client for streaming RPCs (no request timeout on native)
    hardware_streaming: HardwareServiceClient<Transport>,
    scan: ScanServiceClient<Transport>,
    storage: StorageServiceClient<Transport>,
    module: ModuleServiceClient<Transport>,
    run_engine: RunEngineServiceClient<Transport>,
    /// NI DAQ service client for Comedi hardware control
    ni_daq: NiDaqServiceClient<Transport>,
    /// Dedicated NI DAQ streaming client (no request timeout, for stream_analog_input)
    ni_daq_streaming: NiDaqServiceClient<Transport>,
}

/// Maximum message size for gRPC (64 MB for high-resolution camera frames)
/// Must match server's max_encoding_message_size in server.rs
const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

impl DaqClient {
    // =========================================================================
    // Native connection (HTTP/2 via tonic transport)
    // =========================================================================

    /// Connect to the DAQ daemon at the given address with default configuration.
    ///
    /// The address is validated and normalized by `DaemonAddress` before connection.
    /// TLS is automatically enabled for `https://` addresses.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect(address: &DaemonAddress) -> Result<Self> {
        Self::connect_with_config(address, ChannelConfig::default()).await
    }

    /// Connect with custom channel configuration.
    ///
    /// Use `ChannelConfig::fast()` for local/Tailscale connections with quicker
    /// failure detection, or customize timeouts for high-latency networks.
    #[cfg(not(target_arch = "wasm32"))]
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

        // Enable TLS for https:// addresses using system root certificates.
        // Requires the `tls` feature (enabled by default) which pulls in
        // tonic's `tls-roots` for native CA certificate loading.
        #[cfg(feature = "tls")]
        let (endpoint, streaming_endpoint) = if address.is_tls() {
            let tls = tonic::transport::ClientTlsConfig::new();
            (
                endpoint.tls_config(tls.clone())?,
                streaming_endpoint.tls_config(tls)?,
            )
        } else {
            (endpoint, streaming_endpoint)
        };

        #[cfg(not(feature = "tls"))]
        if address.is_tls() {
            anyhow::bail!("TLS address requested but client was built without the `tls` feature");
        }

        let channel = endpoint.connect().await?;
        let streaming_channel = streaming_endpoint.connect().await?;

        Ok(Self {
            control: ControlServiceClient::new(channel.clone()),
            control_streaming: ControlServiceClient::new(streaming_channel.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            // Hardware client needs larger message size for camera frame streaming
            hardware: HardwareServiceClient::new(channel.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            // Dedicated streaming client without request timeout
            hardware_streaming: HardwareServiceClient::new(streaming_channel.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            scan: ScanServiceClient::new(channel.clone()),
            storage: StorageServiceClient::new(channel.clone()),
            module: ModuleServiceClient::new(channel.clone()),
            run_engine: RunEngineServiceClient::new(channel.clone()),
            ni_daq: NiDaqServiceClient::new(channel),
            ni_daq_streaming: NiDaqServiceClient::new(streaming_channel),
        })
    }

    // =========================================================================
    // WASM connection (gRPC-web via browser fetch API)
    // =========================================================================

    /// Connect to the DAQ daemon over gRPC-web (browser-compatible).
    ///
    /// Create a gRPC-web client for browser use.
    ///
    /// The daemon must have gRPC-web support enabled (`tonic_web::enable()`)
    /// and CORS configured to allow the browser's origin.
    ///
    /// Unlike the native `connect()`, this is synchronous — the underlying
    /// `tonic_web_wasm_client::Client` connects lazily on the first RPC call.
    /// Connection errors (network unreachable, CORS failures, etc.) will surface
    /// as errors on the first method call, not at construction time.
    ///
    /// # Arguments
    /// * `base_url` - The daemon's HTTP URL, e.g. `"http://10.0.0.40:50051"`
    #[cfg(target_arch = "wasm32")]
    pub fn connect_web(base_url: &str) -> Self {
        let client = tonic_web_wasm_client::Client::new(base_url.to_string());
        Self {
            control: ControlServiceClient::new(client.clone()),
            control_streaming: ControlServiceClient::new(client.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            hardware: HardwareServiceClient::new(client.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            hardware_streaming: HardwareServiceClient::new(client.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            scan: ScanServiceClient::new(client.clone()),
            storage: StorageServiceClient::new(client.clone()),
            module: ModuleServiceClient::new(client.clone()),
            run_engine: RunEngineServiceClient::new(client.clone()),
            ni_daq: NiDaqServiceClient::new(client.clone()),
            ni_daq_streaming: NiDaqServiceClient::new(client),
        }
    }

    /// Perform a lightweight health check by calling GetDaemonInfo.
    ///
    /// Returns `Ok(())` if the daemon is responsive, `Err` otherwise.
    /// This is used by the ConnectionManager to detect stale connections.
    pub async fn health_check(&mut self) -> Result<()> {
        self.control
            .get_daemon_info(protocol::daq::DaemonInfoRequest {})
            .await?;
        Ok(())
    }

    /// Get daemon information (version, capabilities, etc.)
    pub async fn get_daemon_info(&mut self) -> Result<protocol::daq::DaemonInfoResponse> {
        let response = self.control.get_daemon_info(DaemonInfoRequest {}).await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // NI DAQ Service (Comedi hardware)
    // =========================================================================

    /// Access the NI DAQ service client for single-call RPCs (reads, writes, configure).
    pub fn ni_daq_client(&mut self) -> &mut NiDaqServiceClient<Transport> {
        &mut self.ni_daq
    }

    /// Access the NI DAQ streaming client (no request timeout, for stream_analog_input).
    pub fn ni_daq_streaming_client(&mut self) -> &mut NiDaqServiceClient<Transport> {
        &mut self.ni_daq_streaming
    }

    // =========================================================================
    // Hardware Service
    // =========================================================================

    /// List all devices
    pub async fn list_devices(&mut self) -> Result<Vec<protocol::daq::DeviceInfo>> {
        tracing::debug!("DaqClient::list_devices() - sending gRPC request");
        let response = self
            .hardware
            .list_devices(ListDevicesRequest {
                capability_filter: None,
            })
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "DaqClient::list_devices() - gRPC request failed");
                e
            })?;
        let devices = response.into_inner().devices;
        tracing::debug!(
            device_count = devices.len(),
            "DaqClient::list_devices() - received response"
        );
        Ok(devices)
    }

    /// List all devices with registration failures included
    pub async fn list_devices_full(
        &mut self,
    ) -> Result<(
        Vec<protocol::daq::DeviceInfo>,
        Vec<protocol::daq::RegistrationFailure>,
    )> {
        let response = self
            .hardware
            .list_devices(ListDevicesRequest {
                capability_filter: None,
            })
            .await?;
        let inner = response.into_inner();
        Ok((inner.devices, inner.registration_failures))
    }

    /// Get device state
    pub async fn get_device_state(
        &mut self,
        device_id: &str,
    ) -> Result<protocol::daq::DeviceStateResponse> {
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
    ) -> Result<protocol::daq::MoveResponse> {
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
    ) -> Result<protocol::daq::MoveResponse> {
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
    ) -> Result<protocol::daq::ReadValueResponse> {
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
    pub async fn list_scripts(&mut self) -> Result<Vec<protocol::daq::ScriptInfo>> {
        let response = self.control.list_scripts(ListScriptsRequest {}).await?;
        Ok(response.into_inner().scripts)
    }

    /// List all executions
    pub async fn list_executions(&mut self) -> Result<Vec<protocol::daq::ScriptStatus>> {
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

    /// Stream full spectrum measurements from ControlService (additive path).
    ///
    /// This preserves the existing scalar `StreamMeasurements` behavior by using the
    /// dedicated `StreamSpectra` RPC introduced for vector payloads.
    pub async fn stream_spectra(
        &mut self,
        channels: Vec<String>,
        max_rate_hz: u32,
    ) -> Result<impl futures::Stream<Item = Result<SpectrumPayload, tonic::Status>>> {
        let request = SpectrumStreamRequest {
            channels,
            max_rate_hz,
        };
        let response = self.control_streaming.stream_spectra(request).await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Scan Service
    // =========================================================================

    /// List all scans
    pub async fn list_scans(&mut self) -> Result<Vec<protocol::daq::ScanStatus>> {
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
    ) -> Result<protocol::daq::CreateScanResponse> {
        let response = self
            .scan
            .create_scan(CreateScanRequest {
                config: Some(config),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Start a scan
    pub async fn start_scan(&mut self, scan_id: &str) -> Result<protocol::daq::StartScanResponse> {
        let response = self
            .scan
            .start_scan(StartScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Pause a scan
    pub async fn pause_scan(&mut self, scan_id: &str) -> Result<protocol::daq::PauseScanResponse> {
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
    ) -> Result<protocol::daq::ResumeScanResponse> {
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
    ) -> Result<protocol::daq::StopScanResponse> {
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
    pub async fn get_storage_config(&mut self) -> Result<protocol::daq::StorageConfig> {
        let response = self
            .storage
            .get_storage_config(GetStorageConfigRequest {})
            .await?;
        Ok(response.into_inner())
    }

    /// Get recording status
    pub async fn get_recording_status(&mut self) -> Result<protocol::daq::RecordingStatus> {
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
    ) -> Result<protocol::daq::StartRecordingResponse> {
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
    pub async fn stop_recording(&mut self) -> Result<protocol::daq::StopRecordingResponse> {
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
    pub async fn list_acquisitions(&mut self) -> Result<Vec<protocol::daq::AcquisitionSummary>> {
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
    pub async fn list_module_types(&mut self) -> Result<Vec<protocol::daq::ModuleTypeSummary>> {
        let response = self
            .module
            .list_module_types(ListModuleTypesRequest {
                required_capability: None,
            })
            .await?;
        Ok(response.into_inner().module_types)
    }

    /// List module instances
    pub async fn list_modules(&mut self) -> Result<Vec<protocol::daq::ModuleStatus>> {
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
    ) -> Result<protocol::daq::CreateModuleResponse> {
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
    ) -> Result<protocol::daq::StartModuleResponse> {
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
    ) -> Result<protocol::daq::StopModuleResponse> {
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
    ) -> Result<protocol::daq::AssignDeviceResponse> {
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
    ) -> Result<protocol::daq::StartStreamResponse> {
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
    ) -> Result<protocol::daq::StopStreamResponse> {
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
    ) -> Result<Vec<protocol::daq::ParameterDescriptor>> {
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
    ) -> Result<protocol::daq::ParameterValue> {
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
    ) -> Result<protocol::daq::SetParameterResponse> {
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

    /// Set or clear the favorite flag for a parameter (bd-4wf7).
    pub async fn set_parameter_favorite(
        &mut self,
        device_id: &str,
        parameter_name: &str,
        is_favorite: bool,
    ) -> Result<bool> {
        let response = self
            .hardware
            .set_parameter_favorite(protocol::daq::SetParameterFavoriteRequest {
                device_id: device_id.to_string(),
                parameter_name: parameter_name.to_string(),
                is_favorite,
            })
            .await?;
        Ok(response.into_inner().success)
    }

    /// Get all favorited parameter names for a device (bd-4wf7).
    pub async fn get_parameter_favorites(&mut self, device_id: &str) -> Result<Vec<String>> {
        let response = self
            .hardware
            .get_parameter_favorites(protocol::daq::GetParameterFavoritesRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner().parameter_names)
    }

    /// Load a calibration profile from the daemon's filesystem.
    pub async fn load_calibration_profile(
        &mut self,
        path: &str,
    ) -> Result<protocol::daq::LoadCalibrationProfileResponse> {
        let response = self
            .hardware
            .load_calibration_profile(protocol::daq::LoadCalibrationProfileRequest {
                path: path.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Save a calibration profile to the daemon's filesystem.
    pub async fn save_calibration_profile(
        &mut self,
        path: &str,
        content: &str,
    ) -> Result<protocol::daq::SaveCalibrationProfileResponse> {
        let response = self
            .hardware
            .save_calibration_profile(protocol::daq::SaveCalibrationProfileRequest {
                path: path.to_string(),
                content: content.to_string(),
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
    ) -> Result<protocol::daq::DeviceCommandResponse> {
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
            deadband: 0.001, // Default minimum change threshold
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
                plan_id: None,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Stream documents from plan execution
    pub async fn stream_documents(
        &mut self,
        run_uid: Option<String>,
        doc_types: Vec<i32>,
    ) -> Result<impl futures::Stream<Item = Result<protocol::daq::Document, tonic::Status>>> {
        let response = self
            .run_engine
            .stream_documents(StreamDocumentsRequest { run_uid, doc_types })
            .await?;
        Ok(response.into_inner())
    }

    /// Start the RunEngine to execute queued plans
    pub async fn start_engine(&mut self) -> Result<StartEngineResponse> {
        let response = self.run_engine.start_engine(StartEngineRequest {}).await?;
        Ok(response.into_inner())
    }

    /// Register or update calibration snapshot metadata used by RunEngine readiness gates.
    pub async fn set_calibration_snapshot(
        &mut self,
        snapshot: CalibrationSnapshot,
    ) -> Result<SetCalibrationSnapshotResponse> {
        let response = self
            .run_engine
            .set_calibration_snapshot(SetCalibrationSnapshotRequest {
                snapshot: Some(snapshot),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Clear part or all of a RunEngine calibration snapshot.
    pub async fn clear_calibration_snapshot(
        &mut self,
        device_type: &str,
        clear_radiance_coverage: bool,
        clear_echelle_compatibility: bool,
    ) -> Result<ClearCalibrationSnapshotResponse> {
        let response = self
            .run_engine
            .clear_calibration_snapshot(ClearCalibrationSnapshotRequest {
                device_type: device_type.to_string(),
                clear_radiance_coverage,
                clear_echelle_compatibility,
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Abort the current running plan
    ///
    /// # Arguments
    /// * `run_uid` - Optional run UID to abort. If empty, aborts the current plan.
    pub async fn abort_plan(&mut self, run_uid: Option<&str>) -> Result<AbortPlanResponse> {
        let response = self
            .run_engine
            .abort_plan(AbortPlanRequest {
                run_uid: run_uid.unwrap_or("").to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Pause the RunEngine
    ///
    /// # Arguments
    /// * `defer` - If true, pause at next checkpoint. If false, pause immediately.
    pub async fn pause_engine(&mut self, defer: bool) -> Result<PauseEngineResponse> {
        let response = self
            .run_engine
            .pause_engine(PauseEngineRequest { defer })
            .await?;
        Ok(response.into_inner())
    }

    /// Resume the paused RunEngine
    pub async fn resume_engine(&mut self) -> Result<ResumeEngineResponse> {
        let response = self
            .run_engine
            .resume_engine(ResumeEngineRequest {})
            .await?;
        Ok(response.into_inner())
    }

    /// Get the current RunEngine status
    pub async fn get_engine_status(&mut self) -> Result<EngineStatus> {
        let response = self
            .run_engine
            .get_engine_status(GetEngineStatusRequest {})
            .await?;
        Ok(response.into_inner())
    }

    /// Stream engine state changes (push-based, bd-sz76).
    pub async fn stream_engine_status(
        &mut self,
    ) -> Result<impl futures::Stream<Item = Result<EngineStatus, tonic::Status>>> {
        let response = self
            .run_engine
            .stream_engine_status(StreamEngineStatusRequest {})
            .await?;
        Ok(response.into_inner())
    }

    // =========================================================================
    // Stored Plan Management (bd-yz1w)
    // =========================================================================

    /// Save a plan to persistent storage on the daemon.
    pub async fn save_plan(
        &mut self,
        request: protocol::daq::SavePlanRequest,
    ) -> Result<protocol::daq::SavePlanResponse> {
        let response = self.run_engine.save_plan(request).await?;
        Ok(response.into_inner())
    }

    /// Load a stored plan from the daemon.
    pub async fn load_plan(&mut self, plan_id: &str) -> Result<protocol::daq::LoadPlanResponse> {
        let response = self
            .run_engine
            .load_plan(protocol::daq::LoadPlanRequest {
                plan_id: plan_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// List all saved plans on the daemon.
    pub async fn list_saved_plans(&mut self) -> Result<Vec<protocol::daq::SavedPlanSummary>> {
        let response = self
            .run_engine
            .list_saved_plans(protocol::daq::ListSavedPlansRequest {})
            .await?;
        Ok(response.into_inner().plans)
    }

    /// Delete a saved plan.
    pub async fn delete_saved_plan(
        &mut self,
        plan_id: &str,
    ) -> Result<protocol::daq::DeleteSavedPlanResponse> {
        let response = self
            .run_engine
            .delete_saved_plan(protocol::daq::DeleteSavedPlanRequest {
                plan_id: plan_id.to_string(),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// Queue a stored plan for execution by its plan_id.
    pub async fn queue_stored_plan(
        &mut self,
        plan_id: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<QueuePlanResponse> {
        let response = self
            .run_engine
            .queue_plan(QueuePlanRequest {
                plan_type: String::new(),
                parameters: Default::default(),
                device_mapping: Default::default(),
                metadata,
                plan_id: Some(plan_id.to_string()),
            })
            .await?;
        Ok(response.into_inner())
    }

    /// List recent run records.
    pub async fn list_runs(&mut self, limit: Option<u32>) -> Result<Vec<protocol::daq::RunRecord>> {
        let response = self
            .run_engine
            .list_runs(protocol::daq::ListRunsRequest { limit })
            .await?;
        Ok(response.into_inner().runs)
    }

    // =========================================================================
    // Laser Control (bd-pwjo)
    // =========================================================================

    /// Set shutter state for a device implementing ShutterControl
    ///
    /// # Arguments
    /// * `device_id` - Device ID (e.g., "maitai")
    /// * `open` - true to open shutter, false to close
    ///
    /// # Returns
    /// The actual shutter state after the operation
    pub async fn set_shutter(&mut self, device_id: &str, open: bool) -> Result<bool> {
        let response = self
            .hardware
            .set_shutter(SetShutterRequest {
                device_id: device_id.to_string(),
                open,
            })
            .await?;
        let inner = response.into_inner();
        if inner.success {
            Ok(inner.is_open)
        } else {
            anyhow::bail!("Set shutter failed: {}", inner.error_message)
        }
    }

    /// Get current shutter state for a device
    pub async fn get_shutter(&mut self, device_id: &str) -> Result<bool> {
        let response = self
            .hardware
            .get_shutter(GetShutterRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner().is_open)
    }

    /// Set wavelength for a device implementing WavelengthTunable
    ///
    /// # Arguments
    /// * `device_id` - Device ID (e.g., "maitai")
    /// * `wavelength_nm` - Target wavelength in nanometers (e.g., 800.0)
    ///
    /// # Returns
    /// The actual wavelength after the operation (may differ from requested)
    pub async fn set_wavelength(&mut self, device_id: &str, wavelength_nm: f64) -> Result<f64> {
        let response = self
            .hardware
            .set_wavelength(SetWavelengthRequest {
                device_id: device_id.to_string(),
                wavelength_nm,
            })
            .await?;
        let inner = response.into_inner();
        if inner.success {
            Ok(inner.actual_wavelength_nm)
        } else {
            anyhow::bail!("Set wavelength failed: {}", inner.error_message)
        }
    }

    /// Get current wavelength for a device
    pub async fn get_wavelength(&mut self, device_id: &str) -> Result<f64> {
        let response = self
            .hardware
            .get_wavelength(GetWavelengthRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner().wavelength_nm)
    }

    /// Set emission state for a device implementing EmissionControl
    ///
    /// # Arguments
    /// * `device_id` - Device ID (e.g., "maitai")
    /// * `enabled` - true to enable emission (laser on), false to disable
    ///
    /// # Returns
    /// The actual emission state after the operation
    pub async fn set_emission(&mut self, device_id: &str, enabled: bool) -> Result<bool> {
        let response = self
            .hardware
            .set_emission(SetEmissionRequest {
                device_id: device_id.to_string(),
                enabled,
            })
            .await?;
        let inner = response.into_inner();
        if inner.success {
            Ok(inner.is_enabled)
        } else {
            anyhow::bail!("Set emission failed: {}", inner.error_message)
        }
    }

    /// Get current emission state for a device
    pub async fn get_emission(&mut self, device_id: &str) -> Result<bool> {
        let response = self
            .hardware
            .get_emission(GetEmissionRequest {
                device_id: device_id.to_string(),
            })
            .await?;
        Ok(response.into_inner().is_enabled)
    }
}
