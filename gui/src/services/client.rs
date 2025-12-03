//! gRPC Client for rust-daq daemon
//!
//! Provides a high-level interface to the daemon's gRPC services.

use anyhow::{anyhow, Result};
use rust_daq::grpc::{
    AbortPlanRequest,
    AssignDeviceRequest,
    AxisConfig,
    ConfigureModuleRequest,
    CreateModuleRequest,
    CreateScanRequest,
    DeletePresetRequest,
    DeviceStateSubscribeRequest,
    DeviceStateUpdate,
    Document,
    EngineStatus,
    // Frame streaming types (bd-p6vz)
    FrameData,
    GetEmissionRequest,
    GetEngineStatusRequest,
    GetExposureRequest,
    GetParameterRequest,
    GetPlanTypeInfoRequest,
    GetPluginInfoRequest,
    GetPresetRequest,
    GetShutterRequest,
    GetWavelengthRequest,
    HaltEngineRequest,
    HardwareServiceClient,
    ListDevicesRequest,
    ListModuleTypesRequest,
    ListModulesRequest,
    ListPlanTypesRequest,
    ListPluginInstancesRequest,
    ListPluginsRequest,
    ListPresetsRequest,
    LoadPresetRequest,
    // Module service types (bd-xx7f)
    ModuleServiceClient,
    ModuleStatus as ProtoModuleStatus,
    ModuleTypeSummary,
    MoveRequest,
    PauseEngineRequest,
    PauseModuleRequest,
    PauseScanRequest,
    PlanTypeInfo,
    PlanTypeSummary,
    PluginInfo,
    PluginInstanceSummary,
    // Plugin service types (bd-tr9l)
    PluginServiceClient,
    PluginSummary,
    Preset,
    PresetMetadata,
    // Preset service types (bd-i1c5)
    PresetServiceClient,
    QueuePlanRequest,
    ReadValueRequest,
    ResumeEngineRequest,
    ResumeModuleRequest,
    ResumeScanRequest,
    // RunEngine service types (bd-niy4)
    RunEngineServiceClient,
    SavePresetRequest,
    ScanConfig,
    ScanProgress,
    ScanServiceClient,
    ScanType,
    SetEmissionRequest,
    // Exposure control types (bd-tm0b)
    SetExposureRequest,
    // Parameter control (for settable devices)
    SetParameterRequest,
    // Laser control types (bd-pwjo)
    SetShutterRequest,
    SetWavelengthRequest,
    StartEngineRequest,
    StartModuleRequest,
    StartScanRequest,
    StartStreamRequest,
    StopModuleRequest,
    StopMotionRequest,
    StopScanRequest,
    StopStreamRequest,
    StreamDocumentsRequest,
    StreamFramesRequest,
    StreamScanProgressRequest,
    StreamValuesRequest,
    ValueUpdate,
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
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
    // Metadata from daemon (bd-pwjo)
    pub position_units: Option<String>, // For movable devices (e.g., "degrees", "mm")
    pub reading_units: Option<String>,  // For readable devices (e.g., "W", "mW")
    pub min_position: Option<f64>,
    pub max_position: Option<f64>,
}

/// High-level gRPC client for the rust-daq daemon
#[derive(Clone)]
pub struct DaqClient {
    hardware: HardwareServiceClient<Channel>,
    scan: ScanServiceClient<Channel>,
    module: ModuleServiceClient<Channel>,
    preset: PresetServiceClient<Channel>,
    run_engine: RunEngineServiceClient<Channel>,
    plugin: PluginServiceClient<Channel>,
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

        let hardware = HardwareServiceClient::new(channel.clone());
        let scan = ScanServiceClient::new(channel.clone());
        let module = ModuleServiceClient::new(channel.clone());
        let preset = PresetServiceClient::new(channel.clone());
        let run_engine = RunEngineServiceClient::new(channel.clone());
        let plugin = PluginServiceClient::new(channel);

        Ok(Self {
            hardware,
            scan,
            module,
            preset,
            run_engine,
            plugin,
        })
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
            .map(|d| {
                // Extract metadata fields (bd-pwjo)
                let (position_units, reading_units, min_position, max_position) =
                    if let Some(meta) = d.metadata {
                        (
                            meta.position_units,
                            meta.reading_units,
                            meta.min_position,
                            meta.max_position,
                        )
                    } else {
                        (None, None, None, None)
                    };

                DeviceInfo {
                    id: d.id,
                    name: d.name,
                    driver_type: d.driver_type,
                    is_movable: d.is_movable,
                    is_readable: d.is_readable,
                    is_triggerable: d.is_triggerable,
                    is_frame_producer: d.is_frame_producer,
                    position_units,
                    reading_units,
                    min_position,
                    max_position,
                }
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
                wait_for_completion: None,
                timeout_ms: None,
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
                wait_for_completion: None,
                timeout_ms: None,
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
    #[expect(dead_code, reason = "Part of API, used by future features")]
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
    #[expect(dead_code, reason = "Part of API, used by future features")]
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

    /// Subscribe to device state updates (bd-6uba)
    ///
    /// Returns a receiver channel of `DeviceStateUpdate`.
    pub async fn subscribe_device_state(
        &self,
        device_ids: Vec<String>,
        max_rate_hz: u32,
        include_snapshot: bool,
        last_seen_version: u64,
    ) -> Result<mpsc::Receiver<DeviceStateUpdate>> {
        let mut client = self.hardware.clone();

        let response = client
            .subscribe_device_state(DeviceStateSubscribeRequest {
                device_ids,
                max_rate_hz,
                last_seen_version,
                include_snapshot,
            })
            .await
            .map_err(|e| anyhow!("SubscribeDeviceState RPC failed: {}", e))?;

        let mut stream = response.into_inner();
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(update) => {
                        if tx.send(update).await.is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        let _ = tx
                            .send(DeviceStateUpdate {
                                device_id: "".into(),
                                timestamp_ns: 0,
                                version: 0,
                                is_snapshot: false,
                                fields_json: std::collections::HashMap::new(),
                            })
                            .await;
                        tracing::warn!(
                            error = %status,
                            code = ?status.code(),
                            message = %status.message(),
                            "SubscribeDeviceState stream error"
                        );
                        break;
                    }
                }
            }
            tracing::debug!("SubscribeDeviceState stream ended");
        });

        Ok(rx)
    }

    /// Subscribe with reconnect + resume logic.
    ///
    /// On stream error or closure, it will reconnect with the last seen version.
    /// The returned channel yields updates; when closed, caller can decide to stop.
    pub async fn subscribe_device_state_with_reconnect(
        &self,
        device_ids: Vec<String>,
        max_rate_hz: u32,
        include_snapshot: bool,
    ) -> Result<mpsc::Receiver<DeviceStateUpdate>> {
        let (tx, rx) = mpsc::channel(100);
        let client = self.clone();

        tokio::spawn(async move {
            let mut last_version: u64 = 0;
            loop {
                let result = client
                    .subscribe_device_state(
                        device_ids.clone(),
                        max_rate_hz,
                        include_snapshot,
                        last_version,
                    )
                    .await;

                let mut stream_rx = match result {
                    Ok(r) => r,
                    Err(err) => {
                        tracing::warn!("SubscribeDeviceState connect failed: {}", err);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                };

                while let Some(update) = stream_rx.recv().await {
                    last_version = last_version.max(update.version);
                    if tx.send(update).await.is_err() {
                        return;
                    }
                }

                // Stream ended; retry after short backoff
                tracing::info!("SubscribeDeviceState stream ended, reconnecting");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });

        Ok(rx)
    }

    // =========================================================================
    // Scan Operations (bd-npye)
    // =========================================================================

    /// Create a new scan configuration
    ///
    /// Returns the scan_id and total number of points
    pub async fn create_scan(
        &self,
        device_id: &str,
        start: f64,
        end: f64,
        num_points: u32,
    ) -> Result<(String, u32)> {
        let mut client = self.scan.clone();

        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: device_id.to_string(),
                start_position: start,
                end_position: end,
                num_points,
            }],
            scan_type: ScanType::LineScan.into(),
            acquire_device_ids: vec![],
            triggers_per_point: 1,
            dwell_time_ms: 50.0,
            camera_device_id: None,
            arm_camera: None,
            name: format!("GUI Scan {}", device_id),
            metadata: std::collections::HashMap::new(),
        };

        debug!(
            "CreateScan: {} from {} to {} ({} pts)",
            device_id, start, end, num_points
        );

        let response = client
            .create_scan(CreateScanRequest {
                config: Some(config),
            })
            .await
            .map_err(|e| anyhow!("CreateScan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("CreateScan failed: {}", resp.error_message));
        }

        Ok((resp.scan_id, resp.total_points))
    }

    /// Start executing a created scan
    pub async fn start_scan(&self, scan_id: &str) -> Result<u64> {
        let mut client = self.scan.clone();

        debug!("StartScan: {}", scan_id);

        let response = client
            .start_scan(StartScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("StartScan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StartScan failed: {}", resp.error_message));
        }

        Ok(resp.start_time_ns)
    }

    /// Pause a running scan
    pub async fn pause_scan(&self, scan_id: &str) -> Result<u32> {
        let mut client = self.scan.clone();

        debug!("PauseScan: {}", scan_id);

        let response = client
            .pause_scan(PauseScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("PauseScan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("PauseScan failed"));
        }

        Ok(resp.paused_at_point)
    }

    /// Resume a paused scan
    #[expect(dead_code, reason = "Part of API, used by future features")]
    pub async fn resume_scan(&self, scan_id: &str) -> Result<()> {
        let mut client = self.scan.clone();

        debug!("ResumeScan: {}", scan_id);

        let response = client
            .resume_scan(ResumeScanRequest {
                scan_id: scan_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("ResumeScan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("ResumeScan failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Stop/abort a scan
    pub async fn stop_scan(&self, scan_id: &str) -> Result<u32> {
        let mut client = self.scan.clone();

        debug!("StopScan: {}", scan_id);

        let response = client
            .stop_scan(StopScanRequest {
                scan_id: scan_id.to_string(),
                emergency_stop: false,
            })
            .await
            .map_err(|e| anyhow!("StopScan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StopScan failed: {}", resp.error_message));
        }

        Ok(resp.points_completed)
    }

    /// Stream scan progress updates
    ///
    /// Returns a receiver channel that yields progress updates
    pub async fn stream_scan_progress(
        &self,
        scan_id: &str,
        include_data: bool,
    ) -> Result<mpsc::Receiver<ScanProgress>> {
        let mut client = self.scan.clone();

        let response = client
            .stream_scan_progress(StreamScanProgressRequest {
                scan_id: scan_id.to_string(),
                include_data,
            })
            .await
            .map_err(|e| anyhow!("StreamScanProgress RPC failed: {}", e))?;

        let mut stream = response.into_inner();
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(progress) => {
                        if tx.send(progress).await.is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        tracing::warn!(
                            error = %status,
                            code = ?status.code(),
                            message = %status.message(),
                            "StreamScanProgress error"
                        );
                        break;
                    }
                }
            }
            tracing::debug!("StreamScanProgress stream ended");
        });

        Ok(rx)
    }

    // =========================================================================
    // Frame Streaming Operations (bd-p6vz)
    // =========================================================================

    /// Start frame streaming on a camera device
    ///
    /// Returns Ok(()) on success
    #[expect(dead_code, reason = "Used by CameraPanel (bd-tm0b)")]
    pub async fn start_frame_stream(
        &self,
        device_id: &str,
        frame_count: Option<u32>,
    ) -> Result<()> {
        let mut client = self.hardware.clone();

        debug!("StartStream: {} (count: {:?})", device_id, frame_count);

        let response = client
            .start_stream(StartStreamRequest {
                device_id: device_id.to_string(),
                frame_count,
            })
            .await
            .map_err(|e| anyhow!("StartStream RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StartStream failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Stop frame streaming on a camera device
    ///
    /// Returns the number of frames that were captured
    #[expect(dead_code, reason = "Used by CameraPanel (bd-tm0b)")]
    pub async fn stop_frame_stream(&self, device_id: &str) -> Result<u64> {
        let mut client = self.hardware.clone();

        debug!("StopStream: {}", device_id);

        let response = client
            .stop_stream(StopStreamRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("StopStream RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StopStream failed"));
        }

        Ok(resp.frames_captured)
    }

    /// Stream frames from a camera device
    ///
    /// Returns a receiver channel that yields FrameData updates.
    /// Set include_pixel_data to false for metadata-only streaming (lower bandwidth).
    #[expect(dead_code, reason = "Used by CameraPanel (bd-tm0b)")]
    pub async fn stream_frames(
        &self,
        device_id: &str,
        include_pixel_data: bool,
    ) -> Result<mpsc::Receiver<FrameData>> {
        let mut client = self.hardware.clone();

        info!(
            "StreamFrames: {} (include_pixel_data: {})",
            device_id, include_pixel_data
        );

        let response = client
            .stream_frames(StreamFramesRequest {
                device_id: device_id.to_string(),
                include_pixel_data: Some(include_pixel_data),
            })
            .await
            .map_err(|e| anyhow!("StreamFrames RPC failed: {}", e))?;

        let mut stream = response.into_inner();
        let (tx, rx) = mpsc::channel(32); // Smaller buffer for frames (can be large)

        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(frame) => {
                        if tx.send(frame).await.is_err() {
                            // Receiver dropped, stop streaming
                            break;
                        }
                    }
                    Err(status) => {
                        tracing::warn!(
                            error = %status,
                            code = ?status.code(),
                            message = %status.message(),
                            "StreamFrames error"
                        );
                        break;
                    }
                }
            }
            tracing::debug!("StreamFrames stream ended");
        });

        Ok(rx)
    }

    // =========================================================================
    // Exposure Control Operations (bd-tm0b)
    // =========================================================================

    /// Set camera exposure time
    ///
    /// Returns the actual exposure time (may differ from requested)
    #[expect(dead_code, reason = "Used by CameraPanel")]
    pub async fn set_exposure(&self, device_id: &str, exposure_ms: f64) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("SetExposure: {} = {} ms", device_id, exposure_ms);

        let response = client
            .set_exposure(SetExposureRequest {
                device_id: device_id.to_string(),
                exposure_ms,
            })
            .await
            .map_err(|e| anyhow!("SetExposure RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("SetExposure failed: {}", resp.error_message));
        }

        Ok(resp.actual_exposure_ms)
    }

    /// Get current camera exposure time
    #[expect(dead_code, reason = "Used by CameraPanel")]
    pub async fn get_exposure(&self, device_id: &str) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("GetExposure: {}", device_id);

        let response = client
            .get_exposure(GetExposureRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetExposure RPC failed: {}", e))?;

        let resp = response.into_inner();
        Ok(resp.exposure_ms)
    }

    // =========================================================================
    // Laser Control Operations (bd-pwjo)
    // =========================================================================

    /// Set laser shutter state
    ///
    /// Returns the actual shutter state after the operation
    pub async fn set_shutter(&self, device_id: &str, open: bool) -> Result<bool> {
        let mut client = self.hardware.clone();

        debug!("SetShutter: {} = {}", device_id, open);

        let response = client
            .set_shutter(SetShutterRequest {
                device_id: device_id.to_string(),
                open,
            })
            .await
            .map_err(|e| anyhow!("SetShutter RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.error_message.is_empty() {
            return Err(anyhow!("SetShutter failed: {}", resp.error_message));
        }

        Ok(resp.is_open)
    }

    /// Get current laser shutter state
    pub async fn get_shutter(&self, device_id: &str) -> Result<bool> {
        let mut client = self.hardware.clone();

        debug!("GetShutter: {}", device_id);

        let response = client
            .get_shutter(GetShutterRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetShutter RPC failed: {}", e))?;

        let resp = response.into_inner();
        Ok(resp.is_open)
    }

    /// Set laser wavelength
    ///
    /// Returns the actual wavelength after the operation
    pub async fn set_wavelength(&self, device_id: &str, wavelength_nm: f64) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("SetWavelength: {} = {} nm", device_id, wavelength_nm);

        let response = client
            .set_wavelength(SetWavelengthRequest {
                device_id: device_id.to_string(),
                wavelength_nm,
            })
            .await
            .map_err(|e| anyhow!("SetWavelength RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.error_message.is_empty() {
            return Err(anyhow!("SetWavelength failed: {}", resp.error_message));
        }

        Ok(resp.actual_wavelength_nm)
    }

    /// Get current laser wavelength
    pub async fn get_wavelength(&self, device_id: &str) -> Result<f64> {
        let mut client = self.hardware.clone();

        debug!("GetWavelength: {}", device_id);

        let response = client
            .get_wavelength(GetWavelengthRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetWavelength RPC failed: {}", e))?;

        let resp = response.into_inner();
        Ok(resp.wavelength_nm)
    }

    /// Set laser emission state
    ///
    /// Returns the actual emission state after the operation
    pub async fn set_emission(&self, device_id: &str, enabled: bool) -> Result<bool> {
        let mut client = self.hardware.clone();

        debug!("SetEmission: {} = {}", device_id, enabled);

        let response = client
            .set_emission(SetEmissionRequest {
                device_id: device_id.to_string(),
                enabled,
            })
            .await
            .map_err(|e| anyhow!("SetEmission RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.error_message.is_empty() {
            return Err(anyhow!("SetEmission failed: {}", resp.error_message));
        }

        Ok(resp.is_enabled)
    }

    /// Get current laser emission state
    pub async fn get_emission(&self, device_id: &str) -> Result<bool> {
        let mut client = self.hardware.clone();

        debug!("GetEmission: {}", device_id);

        let response = client
            .get_emission(GetEmissionRequest {
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetEmission RPC failed: {}", e))?;

        let resp = response.into_inner();
        Ok(resp.is_enabled)
    }

    // =========================================================================
    // Module Operations (bd-xx7f)
    // =========================================================================

    /// List available module types
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn list_module_types(&self) -> Result<Vec<ModuleTypeSummary>> {
        let mut client = self.module.clone();

        debug!("ListModuleTypes");

        let response = client
            .list_module_types(ListModuleTypesRequest {
                required_capability: None,
            })
            .await
            .map_err(|e| anyhow!("ListModuleTypes RPC failed: {}", e))?;

        Ok(response.into_inner().module_types)
    }

    /// Create a new module instance
    ///
    /// Returns the module_id on success
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn create_module(
        &self,
        type_id: &str,
        instance_name: &str,
        initial_config: std::collections::HashMap<String, String>,
    ) -> Result<String> {
        let mut client = self.module.clone();

        debug!("CreateModule: type={}, name={}", type_id, instance_name);

        let response = client
            .create_module(CreateModuleRequest {
                type_id: type_id.to_string(),
                instance_name: instance_name.to_string(),
                initial_config,
            })
            .await
            .map_err(|e| anyhow!("CreateModule RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("CreateModule failed: {}", resp.error_message));
        }

        Ok(resp.module_id)
    }

    /// Assign a device to a module role
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn assign_device(
        &self,
        module_id: &str,
        role_id: &str,
        device_id: &str,
    ) -> Result<bool> {
        let mut client = self.module.clone();

        debug!(
            "AssignDevice: module={}, role={}, device={}",
            module_id, role_id, device_id
        );

        let response = client
            .assign_device(AssignDeviceRequest {
                module_id: module_id.to_string(),
                role_id: role_id.to_string(),
                device_id: device_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("AssignDevice RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("AssignDevice failed: {}", resp.error_message));
        }

        Ok(resp.module_ready)
    }

    /// Configure module parameters
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn configure_module(
        &self,
        module_id: &str,
        parameters: std::collections::HashMap<String, String>,
        partial: bool,
    ) -> Result<()> {
        let mut client = self.module.clone();

        debug!("ConfigureModule: module={}", module_id);

        let response = client
            .configure_module(ConfigureModuleRequest {
                module_id: module_id.to_string(),
                parameters,
                partial,
            })
            .await
            .map_err(|e| anyhow!("ConfigureModule RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("ConfigureModule failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Start a module
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn start_module(&self, module_id: &str) -> Result<u64> {
        let mut client = self.module.clone();

        debug!("StartModule: {}", module_id);

        let response = client
            .start_module(StartModuleRequest {
                module_id: module_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("StartModule RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StartModule failed: {}", resp.error_message));
        }

        Ok(resp.start_time_ns)
    }

    /// Stop a module
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn stop_module(&self, module_id: &str, force: bool) -> Result<()> {
        let mut client = self.module.clone();

        debug!("StopModule: {} (force={})", module_id, force);

        let response = client
            .stop_module(StopModuleRequest {
                module_id: module_id.to_string(),
                force,
            })
            .await
            .map_err(|e| anyhow!("StopModule RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StopModule failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Pause a module
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn pause_module(&self, module_id: &str) -> Result<()> {
        let mut client = self.module.clone();

        debug!("PauseModule: {}", module_id);

        let response = client
            .pause_module(PauseModuleRequest {
                module_id: module_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("PauseModule RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("PauseModule failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Resume a paused module
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn resume_module(&self, module_id: &str) -> Result<()> {
        let mut client = self.module.clone();

        debug!("ResumeModule: {}", module_id);

        let response = client
            .resume_module(ResumeModuleRequest {
                module_id: module_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("ResumeModule RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("ResumeModule failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// List all module instances
    #[expect(dead_code, reason = "Used by ModulesPanel")]
    pub async fn list_modules(&self) -> Result<Vec<ProtoModuleStatus>> {
        let mut client = self.module.clone();

        debug!("ListModules");

        let response = client
            .list_modules(ListModulesRequest {
                type_filter: None,
                state_filter: None,
            })
            .await
            .map_err(|e| anyhow!("ListModules RPC failed: {}", e))?;

        Ok(response.into_inner().modules)
    }

    // =========================================================================
    // Preset Operations (bd-i1c5)
    // =========================================================================

    /// List all available presets
    #[expect(dead_code, reason = "Used by PresetPanel")]
    pub async fn list_presets(&self) -> Result<Vec<PresetMetadata>> {
        let mut client = self.preset.clone();

        debug!("ListPresets");

        let response = client
            .list_presets(ListPresetsRequest {})
            .await
            .map_err(|e| anyhow!("ListPresets RPC failed: {}", e))?;

        Ok(response.into_inner().presets)
    }

    /// Get a specific preset by ID
    #[expect(dead_code, reason = "Used by PresetPanel")]
    pub async fn get_preset(&self, preset_id: &str) -> Result<Preset> {
        let mut client = self.preset.clone();

        debug!("GetPreset: {}", preset_id);

        let response = client
            .get_preset(GetPresetRequest {
                preset_id: preset_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetPreset RPC failed: {}", e))?;

        Ok(response.into_inner())
    }

    /// Save a preset (create or update)
    #[expect(dead_code, reason = "Used by PresetPanel")]
    pub async fn save_preset(&self, preset: Preset, overwrite: bool) -> Result<String> {
        let mut client = self.preset.clone();

        debug!("SavePreset: {:?}", preset.meta.as_ref().map(|m| &m.name));

        let response = client
            .save_preset(SavePresetRequest {
                preset: Some(preset),
                overwrite,
            })
            .await
            .map_err(|e| anyhow!("SavePreset RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.saved {
            return Err(anyhow!("SavePreset failed: {}", resp.message));
        }

        Ok(resp.message)
    }

    /// Load and apply a preset to devices
    #[expect(dead_code, reason = "Used by PresetPanel")]
    pub async fn load_preset(&self, preset_id: &str) -> Result<String> {
        let mut client = self.preset.clone();

        debug!("LoadPreset: {}", preset_id);

        let response = client
            .load_preset(LoadPresetRequest {
                preset_id: preset_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("LoadPreset RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.applied {
            return Err(anyhow!("LoadPreset failed: {}", resp.message));
        }

        Ok(resp.message)
    }

    /// Delete a preset
    #[expect(dead_code, reason = "Used by PresetPanel")]
    pub async fn delete_preset(&self, preset_id: &str) -> Result<String> {
        let mut client = self.preset.clone();

        debug!("DeletePreset: {}", preset_id);

        let response = client
            .delete_preset(DeletePresetRequest {
                preset_id: preset_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("DeletePreset RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.deleted {
            return Err(anyhow!("DeletePreset failed: {}", resp.message));
        }

        Ok(resp.message)
    }

    // =========================================================================
    // RunEngine Operations (bd-niy4)
    // =========================================================================

    /// List available plan types
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn list_plan_types(&self) -> Result<Vec<PlanTypeSummary>> {
        let mut client = self.run_engine.clone();

        debug!("ListPlanTypes");

        let response = client
            .list_plan_types(ListPlanTypesRequest {})
            .await
            .map_err(|e| anyhow!("ListPlanTypes RPC failed: {}", e))?;

        Ok(response.into_inner().plan_types)
    }

    /// Get detailed info about a plan type
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn get_plan_type_info(&self, type_id: &str) -> Result<PlanTypeInfo> {
        let mut client = self.run_engine.clone();

        debug!("GetPlanTypeInfo: {}", type_id);

        let response = client
            .get_plan_type_info(GetPlanTypeInfoRequest {
                type_id: type_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetPlanTypeInfo RPC failed: {}", e))?;

        Ok(response.into_inner())
    }

    /// Queue a plan for execution
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn queue_plan(
        &self,
        plan_type: &str,
        parameters: std::collections::HashMap<String, String>,
        device_mapping: std::collections::HashMap<String, String>,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<(String, u32)> {
        let mut client = self.run_engine.clone();

        debug!("QueuePlan: {}", plan_type);

        let response = client
            .queue_plan(QueuePlanRequest {
                plan_type: plan_type.to_string(),
                parameters,
                device_mapping,
                metadata,
            })
            .await
            .map_err(|e| anyhow!("QueuePlan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("QueuePlan failed: {}", resp.error_message));
        }

        Ok((resp.run_uid, resp.queue_position))
    }

    /// Start the run engine
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn start_engine(&self) -> Result<()> {
        let mut client = self.run_engine.clone();

        debug!("StartEngine");

        let response = client
            .start_engine(StartEngineRequest {})
            .await
            .map_err(|e| anyhow!("StartEngine RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("StartEngine failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Pause the run engine
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn pause_engine(&self, defer: bool) -> Result<String> {
        let mut client = self.run_engine.clone();

        debug!("PauseEngine (defer={})", defer);

        let response = client
            .pause_engine(PauseEngineRequest { defer })
            .await
            .map_err(|e| anyhow!("PauseEngine RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("PauseEngine failed"));
        }

        Ok(resp.paused_at)
    }

    /// Resume the run engine
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn resume_engine(&self) -> Result<()> {
        let mut client = self.run_engine.clone();

        debug!("ResumeEngine");

        let response = client
            .resume_engine(ResumeEngineRequest {})
            .await
            .map_err(|e| anyhow!("ResumeEngine RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("ResumeEngine failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Abort the current plan
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn abort_plan(&self, run_uid: Option<&str>) -> Result<()> {
        let mut client = self.run_engine.clone();

        debug!("AbortPlan: {:?}", run_uid);

        let response = client
            .abort_plan(AbortPlanRequest {
                run_uid: run_uid.unwrap_or("").to_string(),
            })
            .await
            .map_err(|e| anyhow!("AbortPlan RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("AbortPlan failed: {}", resp.error_message));
        }

        Ok(())
    }

    /// Emergency halt the engine
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn halt_engine(&self) -> Result<String> {
        let mut client = self.run_engine.clone();

        debug!("HaltEngine");

        let response = client
            .halt_engine(HaltEngineRequest {})
            .await
            .map_err(|e| anyhow!("HaltEngine RPC failed: {}", e))?;

        let resp = response.into_inner();
        Ok(resp.message)
    }

    /// Get engine status
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn get_engine_status(&self) -> Result<EngineStatus> {
        let mut client = self.run_engine.clone();

        debug!("GetEngineStatus");

        let response = client
            .get_engine_status(GetEngineStatusRequest {})
            .await
            .map_err(|e| anyhow!("GetEngineStatus RPC failed: {}", e))?;

        Ok(response.into_inner())
    }

    /// Stream documents from plan execution
    #[expect(dead_code, reason = "Used by ExperimentPanel")]
    pub async fn stream_documents(
        &self,
        run_uid: Option<&str>,
    ) -> Result<mpsc::Receiver<Document>> {
        let mut client = self.run_engine.clone();

        info!("StreamDocuments: {:?}", run_uid);

        let response = client
            .stream_documents(StreamDocumentsRequest {
                run_uid: run_uid.map(|s| s.to_string()),
                doc_types: vec![], // Empty = all types
            })
            .await
            .map_err(|e| anyhow!("StreamDocuments RPC failed: {}", e))?;

        let mut stream = response.into_inner();
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(doc) => {
                        if tx.send(doc).await.is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        tracing::warn!(
                            error = %status,
                            code = ?status.code(),
                            message = %status.message(),
                            "StreamDocuments error"
                        );
                        break;
                    }
                }
            }
            tracing::debug!("StreamDocuments stream ended");
        });

        Ok(rx)
    }

    // =========================================================================
    // Plugin Operations (bd-tr9l)
    // =========================================================================

    /// List available plugins
    #[expect(dead_code, reason = "Used by PluginPanel")]
    pub async fn list_plugins(&self) -> Result<Vec<PluginSummary>> {
        let mut client = self.plugin.clone();

        debug!("ListPlugins");

        let response = client
            .list_plugins(ListPluginsRequest {
                driver_type_filter: None,
            })
            .await
            .map_err(|e| anyhow!("ListPlugins RPC failed: {}", e))?;

        Ok(response.into_inner().plugins)
    }

    /// Get detailed plugin information
    #[expect(dead_code, reason = "Used by PluginPanel")]
    pub async fn get_plugin_info(&self, plugin_id: &str) -> Result<PluginInfo> {
        let mut client = self.plugin.clone();

        debug!("GetPluginInfo: {}", plugin_id);

        let response = client
            .get_plugin_info(GetPluginInfoRequest {
                plugin_id: plugin_id.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetPluginInfo RPC failed: {}", e))?;

        Ok(response.into_inner())
    }

    /// List spawned plugin instances (actual devices)
    #[expect(dead_code, reason = "Used by PluginPanel")]
    pub async fn list_plugin_instances(&self) -> Result<Vec<PluginInstanceSummary>> {
        let mut client = self.plugin.clone();

        debug!("ListPluginInstances");

        let response = client
            .list_plugin_instances(ListPluginInstancesRequest {
                plugin_id: None, // List all instances regardless of plugin type
            })
            .await
            .map_err(|e| anyhow!("ListPluginInstances RPC failed: {}", e))?;

        Ok(response.into_inner().instances)
    }

    /// Set a parameter on a settable device
    #[expect(dead_code, reason = "Used by PluginPanel")]
    pub async fn set_parameter(
        &self,
        device_id: &str,
        parameter_name: &str,
        value: &str,
    ) -> Result<String> {
        let mut client = self.hardware.clone();

        debug!("SetParameter: {}.{} = {}", device_id, parameter_name, value);

        let response = client
            .set_parameter(SetParameterRequest {
                device_id: device_id.to_string(),
                parameter_name: parameter_name.to_string(),
                value: value.to_string(),
            })
            .await
            .map_err(|e| anyhow!("SetParameter RPC failed: {}", e))?;

        let resp = response.into_inner();
        if !resp.success {
            return Err(anyhow!("SetParameter failed: {}", resp.error_message));
        }

        Ok(resp.actual_value)
    }

    /// Get a parameter from a device
    #[expect(dead_code, reason = "Used by PluginPanel")]
    pub async fn get_parameter(&self, device_id: &str, parameter_name: &str) -> Result<String> {
        let mut client = self.hardware.clone();

        debug!("GetParameter: {}.{}", device_id, parameter_name);

        let response = client
            .get_parameter(GetParameterRequest {
                device_id: device_id.to_string(),
                parameter_name: parameter_name.to_string(),
            })
            .await
            .map_err(|e| anyhow!("GetParameter RPC failed: {}", e))?;

        let resp = response.into_inner();
        Ok(resp.value)
    }
}
