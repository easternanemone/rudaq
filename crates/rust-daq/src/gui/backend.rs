//! Backend thread with tokio runtime for gRPC communication.
//!
//! The backend runs in a dedicated std::thread with its own tokio runtime.
//! It manages the gRPC client and stream tasks, communicating with the UI
//! thread via channels.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use tokio::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use tonic::transport::{Channel, Endpoint};
#[cfg(target_arch = "wasm32")]
use tonic_web_wasm_client::Client as WasmClient;
// struct WasmClient;
use tracing::{debug, error, info, warn};

use daq_proto::daq::hardware_service_client::HardwareServiceClient;
use daq_proto::daq::{
    DeviceInfo as ProtoDeviceInfo, DeviceStateSubscribeRequest, ListDevicesRequest,
    ListParametersRequest, MoveRequest, ParameterDescriptor as ProtoParameterDescriptor,
    ReadValueRequest, SetParameterRequest,
};

use super::channels::BackendHandle;
use super::platform;
use super::types::{
    BackendCommand, BackendEvent, BackendMetrics, ConnectionStatus, DeviceInfo,
    DeviceStateSnapshot, ParameterDescriptor, ParameterType,
};
use futures::StreamExt;

/// Streaming metrics snapshot pulled from gRPC frames.
#[derive(Debug, Clone, Default)]
struct StreamMetricsSnapshot {
    current_fps: f64,
    frames_sent: u64,
    frames_dropped: u64,
    avg_latency_ms: f64,
}

/// Statistics tracked by the backend.
#[derive(Debug, Default)]
struct BackendStats {
    frames_dropped: AtomicU64,
    #[allow(dead_code)]
    plots_dropped: AtomicU64,
    stream_restarts: AtomicU64,
    stream_metrics: Mutex<StreamMetricsSnapshot>,
}

#[cfg(not(target_arch = "wasm32"))]
type ClientType = Channel;

#[cfg(target_arch = "wasm32")]
type ClientType = WasmClient;

/// Backend state machine.
pub struct Backend {
    handle: BackendHandle,
    stats: Arc<BackendStats>,
    client: Option<HardwareServiceClient<ClientType>>,
    #[cfg(not(target_arch = "wasm32"))]
    endpoint: Option<Endpoint>,
    shutdown: bool,
    #[allow(dead_code)]
    last_rtt_check: Instant,
    /// Handle to the state streaming task
    state_stream_task: Option<platform::JoinHandle<()>>,
    /// Channel to stop the state stream
    state_stream_stop_tx: Option<mpsc::Sender<()>>,
    /// Handle to the video stream task
    video_stream_task: Option<platform::JoinHandle<()>>,
    /// Channel to stop the video stream
    video_stream_stop_tx: Option<mpsc::Sender<()>>,
}

impl Backend {
    /// Create a new backend with the given channel handle.
    pub fn new(handle: BackendHandle) -> Self {
        Self {
            handle,
            stats: Arc::new(BackendStats::default()),
            client: None,
            #[cfg(not(target_arch = "wasm32"))]
            endpoint: None,
            shutdown: false,
            last_rtt_check: Instant::now(),
            state_stream_task: None,
            state_stream_stop_tx: None,
            video_stream_task: None,
            video_stream_stop_tx: None,
        }
    }

    /// Run the backend main loop (call from tokio runtime).
    pub async fn run(mut self) {
        info!("Backend starting");

        // Use interval instead of sleep for guaranteed periodic metrics updates
        // Skip missed ticks if commands take longer than expected
        // Use interval instead of sleep for guaranteed periodic metrics updates
        // Skip missed ticks if commands take longer than expected
        let mut metrics_interval = platform::interval(Duration::from_millis(100));
        // metrics_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip); // Handled in platform::interval

        loop {
            // Process commands from UI
            tokio::select! {
                Some(cmd) = self.handle.recv_command() => {
                    if !self.process_command(cmd).await {
                        break; // Shutdown requested
                    }
                }
                // Periodic metrics update (interval doesn't reset on other branches)
                _ = metrics_interval.next() => {
                    self.update_metrics();
                }
            }
        }

        info!("Backend shutting down");
        self.shutdown = true;
    }

    /// Process a command from the UI. Returns false if shutdown requested.
    async fn process_command(&mut self, cmd: BackendCommand) -> bool {
        // Trace command reception
        if let BackendCommand::Connect { .. } = cmd {
            log::error!("Debug: Backend received Connect command [LOG]");
        }
        match cmd {
            BackendCommand::Connect { address } => {
                self.connect(&address).await;
            }
            BackendCommand::Disconnect => {
                self.disconnect().await;
            }
            BackendCommand::RefreshDevices => {
                self.refresh_devices().await;
            }
            BackendCommand::FetchParameters { device_id } => {
                self.list_parameters(&device_id).await;
            }
            BackendCommand::MoveAbsolute {
                device_id,
                position,
            } => {
                self.move_absolute(&device_id, position).await;
            }
            BackendCommand::MoveRelative {
                device_id,
                distance,
            } => {
                self.move_relative(&device_id, distance).await;
            }
            BackendCommand::ReadValue { device_id } => {
                self.read_value(&device_id).await;
            }
            BackendCommand::SetParameter {
                device_id,
                name,
                value,
            } => {
                self.set_parameter(&device_id, &name, &value).await;
            }
            BackendCommand::StartStateStream { device_ids } => {
                self.start_state_stream(device_ids).await;
            }
            BackendCommand::StopStateStream => {
                self.stop_state_stream().await;
            }
            BackendCommand::StartVideoStream { device_id } => {
                self.start_video_stream(device_id).await;
            }
            BackendCommand::StopVideoStream => {
                self.stop_video_stream().await;
            }
            BackendCommand::Shutdown => {
                self.stop_state_stream().await;
                self.stop_video_stream().await;
                return false;
            }
        }
        true
    }

    /// Connect to the daemon at the given address.
    async fn connect(&mut self, address: &str) {
        info!("Connecting to daemon at {}", address);
        self.send_connection_status(ConnectionStatus::Connecting);

        let addr = if address.starts_with("http") {
            address.to_string()
        } else {
            format!("http://{}", address)
        };

        #[cfg(not(target_arch = "wasm32"))]
        match Endpoint::from_shared(addr.clone()) {
            Ok(endpoint) => {
                let endpoint = endpoint
                    .connect_timeout(Duration::from_secs(5))
                    .timeout(Duration::from_secs(30));

                match endpoint.connect().await {
                    Ok(channel) => {
                        self.client = Some(HardwareServiceClient::new(channel));
                        self.endpoint = Some(endpoint);
                        self.send_connection_status(ConnectionStatus::Connected);
                        info!("Connected to daemon");
                        self.refresh_devices().await;
                    }
                    Err(e) => {
                        error!("Failed to connect: {}", e);
                        self.send_connection_status(ConnectionStatus::Failed {
                            reason: e.to_string(),
                        });
                        self.handle.send_event(BackendEvent::Error {
                            message: format!("Connection failed: {}", e),
                        });
                    }
                }
            }
            Err(e) => {
                error!("Invalid address: {}", e);
                self.send_connection_status(ConnectionStatus::Failed {
                    reason: format!("Invalid address: {}", e),
                });
            }
        }

        #[cfg(target_arch = "wasm32")]
        #[cfg(target_arch = "wasm32")]
        {
            log::error!("Debug: Starting WASM connect to {} [LOG]", addr);
            let client = WasmClient::new(addr);
            log::error!("Debug: WasmClient created [LOG]");
            self.client = Some(HardwareServiceClient::new(client));
            self.send_connection_status(ConnectionStatus::Connected);
            log::error!("Debug: Connected to daemon (WASM) [LOG]");
            self.refresh_devices().await;
            log::error!("Debug: refresh_devices completed [LOG]");
        }
    }

    /// Disconnect from the daemon.
    async fn disconnect(&mut self) {
        info!("Disconnecting from daemon");

        // Stop any running state stream first
        self.stop_state_stream().await;

        self.client = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.endpoint = None;
        }
        self.send_connection_status(ConnectionStatus::Disconnected);

        // Clear state
        self.handle.update_state(DeviceStateSnapshot::default());
    }

    /// Refresh the device list from the daemon.
    async fn refresh_devices(&mut self) {
        let Some(client) = &mut self.client else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        debug!("Refreshing device list");
        let start = Instant::now();

        let rpc_call = client.list_devices(ListDevicesRequest {
            capability_filter: None,
        });

        log::error!("Debug: Waiting for response from list_devices... [LOG]");
        let response = tokio::select! {
            res = rpc_call => {
                log::error!("Debug: list_devices returned response (or error) [LOG]");
                res
            },
            _ = platform::sleep(Duration::from_secs(5)) => {
                log::error!("Debug: list_devices timed out (5s) [LOG]");
                Err(tonic::Status::deadline_exceeded("Timeout waiting for list_devices"))
            }
        };

        match response {
            Ok(response) => {
                let elapsed = start.elapsed();
                debug!("Device list received in {:?}", elapsed);

                let devices: Vec<DeviceInfo> = response
                    .into_inner()
                    .devices
                    .into_iter()
                    .map(proto_to_device_info)
                    .collect();

                // Update state snapshot with device list
                let mut state = DeviceStateSnapshot::default();
                state.is_connected = true;
                for device in &devices {
                    state.devices.insert(
                        device.id.clone(),
                        super::types::DeviceState {
                            device_id: device.id.clone(),
                            fields: HashMap::new(),
                            version: 0,
                            updated_at: Some(Instant::now()),
                        },
                    );
                }
                self.handle.update_state(state);

                self.handle
                    .send_event(BackendEvent::DevicesRefreshed { devices });
            }
            Err(e) => {
                error!("Failed to refresh devices: {}", e);
                self.handle.send_event(BackendEvent::Error {
                    message: format!("Failed to refresh devices: {}", e),
                });
            }
        }
    }

    /// Read a scalar value from a device.
    async fn read_value(&mut self, device_id: &str) {
        let Some(client) = &mut self.client else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        debug!("Reading value from {}", device_id);

        match client
            .read_value(ReadValueRequest {
                device_id: device_id.to_string(),
            })
            .await
        {
            Ok(response) => {
                let body = response.into_inner();
                if body.success {
                    self.handle.send_event(BackendEvent::ValueRead {
                        device_id: device_id.to_string(),
                        value: body.value,
                        units: body.units,
                    });
                } else {
                    self.handle.send_event(BackendEvent::Error {
                        message: body.error_message,
                    });
                }
            }
            Err(e) => {
                error!("Failed to read value: {}", e);
                self.handle.send_event(BackendEvent::Error {
                    message: format!("Failed to read value: {}", e),
                });
            }
        }
    }

    /// Move a device to absolute position.
    async fn move_absolute(&mut self, device_id: &str, position: f64) {
        let Some(client) = &mut self.client else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        debug!("Moving {} to {}", device_id, position);

        match client
            .move_absolute(MoveRequest {
                device_id: device_id.to_string(),
                value: position,
                wait_for_completion: Some(false), // Non-blocking
                timeout_ms: Some(30000),
            })
            .await
        {
            Ok(response) => {
                let body = response.into_inner();
                if !body.success {
                    self.handle.send_event(BackendEvent::Error {
                        message: body.error_message,
                    });
                }
            }
            Err(e) => {
                error!("Failed to move: {}", e);
                self.handle.send_event(BackendEvent::Error {
                    message: format!("Failed to move: {}", e),
                });
            }
        }
    }

    /// Move a device by relative distance.
    async fn move_relative(&mut self, device_id: &str, distance: f64) {
        let Some(client) = &mut self.client else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        debug!("Moving {} by {}", device_id, distance);

        match client
            .move_relative(MoveRequest {
                device_id: device_id.to_string(),
                value: distance,
                wait_for_completion: Some(false),
                timeout_ms: Some(30000),
            })
            .await
        {
            Ok(response) => {
                let body = response.into_inner();
                if !body.success {
                    self.handle.send_event(BackendEvent::Error {
                        message: body.error_message,
                    });
                }
            }
            Err(e) => {
                error!("Failed to move: {}", e);
                self.handle.send_event(BackendEvent::Error {
                    message: format!("Failed to move: {}", e),
                });
            }
        }
    }

    /// Set a parameter on a device.
    async fn set_parameter(&mut self, device_id: &str, name: &str, value: &str) {
        let Some(client) = &mut self.client else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        debug!("Setting {}.{} = {}", device_id, name, value);

        match client
            .set_parameter(SetParameterRequest {
                device_id: device_id.to_string(),
                parameter_name: name.to_string(),
                value: value.to_string(),
            })
            .await
        {
            Ok(response) => {
                let body = response.into_inner();
                if !body.success {
                    self.handle.send_event(BackendEvent::Error {
                        message: body.error_message,
                    });
                }
            }
            Err(e) => {
                error!("Failed to set parameter: {}", e);
                self.handle.send_event(BackendEvent::Error {
                    message: format!("Failed to set parameter: {}", e),
                });
            }
        }
    }

    /// List parameters for a device.
    async fn list_parameters(&mut self, device_id: &str) {
        let Some(client) = &mut self.client else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        debug!("Listing parameters for {}", device_id);

        match client
            .list_parameters(ListParametersRequest {
                device_id: device_id.to_string(),
            })
            .await
        {
            Ok(response) => {
                let parameters: Vec<ParameterDescriptor> = response
                    .into_inner()
                    .parameters
                    .into_iter()
                    .map(proto_to_parameter_descriptor)
                    .collect();

                debug!("Received {} parameters for {}", parameters.len(), device_id);
                self.handle.send_event(BackendEvent::ParametersFetched {
                    device_id: device_id.to_string(),
                    parameters,
                });
            }
            Err(e) => {
                error!("Failed to list parameters for {}: {}", device_id, e);
                self.handle.send_event(BackendEvent::Error {
                    message: format!("Failed to list parameters: {}", e),
                });
            }
        }
    }

    /// Start streaming device state updates.
    async fn start_state_stream(&mut self, device_ids: Vec<String>) {
        // Stop existing stream if any
        self.stop_state_stream().await;

        let Some(client) = self.client.clone() else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        info!(
            "Starting state stream for {} devices",
            if device_ids.is_empty() {
                "all".to_string()
            } else {
                device_ids.len().to_string()
            }
        );

        // Create stop channel
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        self.state_stream_stop_tx = Some(stop_tx);

        // Clone what we need for the task
        let event_tx = self.handle.event_tx.clone();
        let state_tx = self.handle.state_tx.clone(); // Use watch channel for state updates
        let stats = self.stats.clone();

        // Spawn the streaming task
        let task = platform::spawn(async move {
            let mut client = client;
            let mut backoff_ms = 100u64;
            const MAX_BACKOFF_MS: u64 = 30_000;

            loop {
                // Subscribe to device state
                let request = DeviceStateSubscribeRequest {
                    device_ids: device_ids.clone(),
                    max_rate_hz: 30, // 30 Hz update rate for smooth UI
                    last_seen_version: 0,
                    include_snapshot: true,
                };

                match client.subscribe_device_state(request).await {
                    Ok(response) => {
                        // Reset backoff on success
                        backoff_ms = 100;

                        let _ = event_tx.try_send(BackendEvent::StateStreamStarted);

                        let mut stream = response.into_inner();

                        loop {
                            tokio::select! {
                                // Check for stop signal
                                _ = stop_rx.recv() => {
                                    info!("State stream stop requested");
                                    let _ = event_tx.try_send(BackendEvent::StateStreamStopped);
                                    return;
                                }
                                // Receive stream updates
                                update = stream.message() => {
                                    match update {
                                        Ok(Some(state)) => {
                                            // Update watch channel (never blocks, always latest)
                                            state_tx.send_modify(|snapshot| {
                                                let device_state = snapshot.devices
                                                    .entry(state.device_id.clone())
                                                    .or_insert_with(|| super::types::DeviceState {
                                                        device_id: state.device_id.clone(),
                                                        fields: HashMap::new(),
                                                        version: 0,
                                                        updated_at: None,
                                                    });
                                                device_state.fields.extend(state.fields_json);
                                                device_state.version = state.version;
                                                device_state.updated_at = Some(Instant::now());
                                                snapshot.is_connected = true;
                                            });
                                        }
                                        Ok(None) => {
                                            // Stream ended
                                            debug!("State stream ended");
                                            break;
                                        }
                                        Err(e) => {
                                            error!("State stream error: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to subscribe to device state: {}", e);
                        let _ = event_tx.try_send(BackendEvent::Error {
                            message: format!("State stream failed: {}", e),
                        });
                    }
                }

                // Check for stop before reconnecting
                tokio::select! {
                    _ = stop_rx.recv() => {
                        info!("State stream stop requested during backoff");
                        let _ = event_tx.try_send(BackendEvent::StateStreamStopped);
                        return;
                    }
                    _ = platform::sleep(Duration::from_millis(backoff_ms)) => {
                        // Continue to reconnect
                    }
                }

                // Exponential backoff
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
                stats.stream_restarts.fetch_add(1, Ordering::Relaxed);
                warn!("Reconnecting state stream (backoff: {}ms)", backoff_ms);
            }
        });

        self.state_stream_task = Some(task);
    }

    /// Stop the state stream.
    async fn stop_state_stream(&mut self) {
        if let Some(stop_tx) = self.state_stream_stop_tx.take() {
            let _ = stop_tx.send(()).await;
        }
        if let Some(task) = self.state_stream_task.take() {
            // Give it a moment to gracefully stop
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = tokio::time::timeout(Duration::from_millis(500), task).await;
            }
        }
    }

    /// Start streaming video frames from a device.
    async fn start_video_stream(&mut self, device_id: String) {
        // Stop existing stream if any
        self.stop_video_stream().await;

        let Some(client) = self.client.clone() else {
            self.handle.send_event(BackendEvent::Error {
                message: "Not connected to daemon".to_string(),
            });
            return;
        };

        info!("Starting video stream for {}", device_id);

        // Create stop channel
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        self.video_stream_stop_tx = Some(stop_tx);

        let event_tx = self.handle.event_tx.clone();
        let stats = self.stats.clone();
        let device_id_task = device_id.clone();

        // Spawn streaming task
        let task = platform::spawn(async move {
            let mut client = client;
            let mut backoff_ms = 100u64;
            const MAX_BACKOFF_MS: u64 = 5_000;

            loop {
                let request = daq_proto::daq::StreamFramesRequest {
                    device_id: device_id_task.clone(),
                    max_fps: 30,
                    quality: daq_proto::daq::StreamQuality::Full.into(),
                };

                match client.stream_frames(request).await {
                    Ok(response) => {
                        backoff_ms = 100;
                        let mut stream = response.into_inner();

                        loop {
                            tokio::select! {
                                _ = stop_rx.recv() => {
                                    info!("Video stream stop requested");
                                    return;
                                }
                                update = stream.message() => {
                                    match update {
                                        Ok(Some(frame)) => {
                                            if let Some(metrics) = frame.metrics.as_ref() {
                                                if let Ok(mut guard) = stats.stream_metrics.lock()
                                                {
                                                    guard.current_fps = metrics.current_fps;
                                                    guard.frames_sent = metrics.frames_sent;
                                                    guard.frames_dropped = metrics.frames_dropped;
                                                    guard.avg_latency_ms = metrics.avg_latency_ms;
                                                }
                                            }

                                            // Handle frame
                                            // FrameData has width, height, data
                                            let w = frame.width as usize;
                                            let h = frame.height as usize;

                                            // Only send if we have data to avoid empty updates
                                            if !frame.data.is_empty() {
                                                let _ = event_tx.try_send(BackendEvent::ImageReceived {
                                                    device_id: device_id_task.clone(),
                                                    size: [w, h],
                                                    data: frame.data,
                                                });
                                            }
                                        }
                                        Ok(None) => {
                                            debug!("Video stream ended");
                                            break;
                                        }
                                        Err(e) => {
                                            error!("Video stream error: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to subscribe to video stream: {}", e);
                    }
                }

                // Check stop before retry
                tokio::select! {
                    _ = stop_rx.recv() => {
                         info!("Video stream stop requested during backoff");
                         return;
                    }
                    _ = platform::sleep(Duration::from_millis(backoff_ms)) => {}
                }

                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
                stats.stream_restarts.fetch_add(1, Ordering::Relaxed); // Reuse metric? Or add new one? Reusing fine for now.
            }
        });

        self.video_stream_task = Some(task);
    }

    /// Stop the video stream.
    async fn stop_video_stream(&mut self) {
        if let Some(stop_tx) = self.video_stream_stop_tx.take() {
            let _ = stop_tx.send(()).await;
        }
        if let Some(task) = self.video_stream_task.take() {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = tokio::time::timeout(Duration::from_millis(500), task).await;
            }
        }
    }

    /// Send connection status to UI via state snapshot.
    fn send_connection_status(&self, status: ConnectionStatus) {
        self.handle
            .send_event(BackendEvent::ConnectionChanged { status });
    }

    /// Update and send metrics to UI.
    fn update_metrics(&mut self) {
        let stream_snapshot = self
            .stats
            .stream_metrics
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let metrics = BackendMetrics {
            frames_dropped: self.stats.frames_dropped.load(Ordering::Relaxed),
            stream_restarts: self.stats.stream_restarts.load(Ordering::Relaxed),
            stream_current_fps: stream_snapshot.current_fps,
            stream_frames_sent: stream_snapshot.frames_sent,
            stream_frames_dropped: stream_snapshot.frames_dropped,
            stream_avg_latency_ms: stream_snapshot.avg_latency_ms,
            is_connected: self.client.is_some(),
            ..Default::default()
        };
        self.handle.update_metrics(metrics);
    }
}

/// Convert proto DeviceInfo to UI DeviceInfo.
fn proto_to_device_info(proto: ProtoDeviceInfo) -> DeviceInfo {
    // Build capabilities list from boolean flags
    let mut capabilities = Vec::new();
    if proto.is_movable {
        capabilities.push("Movable".to_string());
    }
    if proto.is_readable {
        capabilities.push("Readable".to_string());
    }
    if proto.is_triggerable {
        capabilities.push("Triggerable".to_string());
    }
    if proto.is_frame_producer {
        capabilities.push("FrameProducer".to_string());
    }
    if proto.is_exposure_controllable {
        capabilities.push("ExposureControl".to_string());
    }
    if proto.is_shutter_controllable {
        capabilities.push("ShutterControl".to_string());
    }
    if proto.is_wavelength_tunable {
        capabilities.push("WavelengthTunable".to_string());
    }
    if proto.is_emission_controllable {
        capabilities.push("EmissionControl".to_string());
    }

    DeviceInfo {
        id: proto.id,
        name: proto.name,
        driver_type: proto.driver_type,
        capabilities,
        is_connected: true,
    }
}

/// Convert proto ParameterDescriptor to UI ParameterDescriptor.
fn proto_to_parameter_descriptor(proto: ProtoParameterDescriptor) -> ParameterDescriptor {
    ParameterDescriptor {
        device_id: proto.device_id,
        name: proto.name,
        description: proto.description,
        dtype: ParameterType::from(proto.dtype.as_str()),
        units: proto.units,
        readable: proto.readable,
        writable: proto.writable,
        min_value: proto.min_value,
        max_value: proto.max_value,
        enum_values: proto.enum_values,
        current_value: None, // Will be populated from state stream
    }
}

/// Spawn the backend in a dedicated thread with its own tokio runtime.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_backend(handle: BackendHandle) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("gui-backend".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            rt.block_on(async move {
                let backend = Backend::new(handle);
                backend.run().await;
            });
        })
        .expect("Failed to spawn backend thread")
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_backend(handle: BackendHandle) {
    log::error!("Debug: spawn_backend called (backend.rs) [LOG]");
    wasm_bindgen_futures::spawn_local(async move {
        log::error!("Debug: spawn_backend async block started (backend.rs) [LOG]");
        let backend = Backend::new(handle);
        log::error!("Debug: Backend created (backend.rs) [LOG]");
        backend.run().await;
        log::error!("Debug: Backend run finished (unexpected) (backend.rs) [LOG]");
    });
}
