//! HardwareService implementation for direct device control (bd-4x6q)
//!
//! This module provides gRPC endpoints for direct hardware manipulation,
//! bypassing the scripting layer. It connects to the DeviceRegistry for
//! capability-based access to hardware devices.

use crate::grpc::proto::{
    ArmRequest,
    ArmResponse,
    CompressionType,
    DeviceCommandRequest,
    DeviceCommandResponse,
    DeviceInfo,
    DeviceMetadata as ProtoDeviceMetadata,
    DeviceStateRequest,
    DeviceStateResponse,
    DeviceStateSubscribeRequest,
    DeviceStateUpdate,
    FrameData,
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
    ListParametersRequest,
    ListParametersResponse,
    MoveRequest,
    MoveResponse,
    ObservableValue,
    ParameterChange,
    ParameterDescriptor,
    ParameterValue,
    PositionUpdate,
    ReadValueRequest,
    ReadValueResponse,
    RegistrationFailure as ProtoRegistrationFailure,
    SetEmissionRequest,
    SetEmissionResponse,
    SetExposureRequest,
    SetExposureResponse,
    SetParameterRequest,
    SetParameterResponse,
    // Laser control types (bd-pwjo)
    SetShutterRequest,
    SetShutterResponse,
    SetWavelengthRequest,
    SetWavelengthResponse,
    StageDeviceRequest,
    StageDeviceResponse,
    StartStreamRequest,
    StartStreamResponse,
    StopMotionRequest,
    StopMotionResponse,
    StopStreamRequest,
    StopStreamResponse,
    // Stream quality for server-side downsampling
    StreamFramesRequest,
    StreamObservablesRequest,
    StreamParameterChangesRequest,
    StreamPositionRequest,
    StreamQuality,
    StreamValuesRequest,
    StreamingMetrics,
    TriggerRequest,
    TriggerResponse,
    UnstageDeviceRequest,
    UnstageDeviceResponse,
    ValueUpdate,
    WaitSettledRequest,
    WaitSettledResponse,
    hardware_service_server::HardwareService,
};
use anyhow::Error as AnyError;
use daq_core::capabilities::FrameObserver;
use daq_core::data::FrameView;
use daq_core::observable::Observable;
use daq_core::parameter::Parameter;
use daq_hardware::registry::{Capability, DeviceRegistry};
use daq_proto::downsample::{downsample_2x2, downsample_4x4};
use serde_json;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, interval};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::instrument;

// =============================================================================
// Frame Observer for gRPC Streaming (bd-0dax.6.3)
// =============================================================================

/// Internal frame data packet sent through the observer channel.
///
/// Contains pre-processed frame data ready for gRPC transmission.
struct ObserverFramePacket {
    data: Vec<u8>,
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    timestamp_ns: u64,
    exposure_ms: Option<f64>,
    roi_x: u32,
    roi_y: u32,
    temperature_c: Option<f64>,
    binning: Option<(u16, u16)>,
}

/// Observer that sends frames to gRPC stream (bd-0dax.6.3).
///
/// This observer receives `FrameView` references from the frame loop and
/// forwards them (after optional downsampling) to a gRPC client via an
/// mpsc channel.
///
/// # Contract
///
/// - `on_frame()` MUST NOT block - uses `try_send()` with bounded channel
/// - Frame data is copied during `on_frame()` (required - can't hold reference)
/// - Backpressure is handled by dropping frames when channel is full
///
/// # Quality Modes
///
/// - `Full`: No downsampling, full resolution frames
/// - `Preview`: 2x2 binning, ~75% bandwidth reduction
/// - `Fast`: 4x4 binning, ~94% bandwidth reduction
struct GrpcStreamObserver {
    /// Channel sender for frame packets (bounded to handle backpressure)
    tx: tokio::sync::mpsc::Sender<ObserverFramePacket>,
    /// Quality setting for server-side downsampling
    quality: StreamQuality,
    /// Device ID for logging
    device_id: String,
    /// Frame counter for logging
    frames_received: AtomicU64,
    /// Frames dropped due to backpressure
    frames_dropped: AtomicU64,
}

impl GrpcStreamObserver {
    /// Create a new gRPC stream observer.
    fn new(
        tx: tokio::sync::mpsc::Sender<ObserverFramePacket>,
        quality: StreamQuality,
        device_id: String,
    ) -> Self {
        Self {
            tx,
            quality,
            device_id,
            frames_received: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
        }
    }
}

impl FrameObserver for GrpcStreamObserver {
    fn on_frame(&self, frame: &FrameView<'_>) {
        let frame_count = self.frames_received.fetch_add(1, Ordering::Relaxed);

        // Log early frames for debugging
        if frame_count < 10 {
            tracing::debug!(
                device_id = %self.device_id,
                frame_number = frame.frame_number,
                width = frame.width,
                height = frame.height,
                quality = ?self.quality,
                "GrpcStreamObserver received frame (early frame debug)"
            );
        }

        // Apply server-side downsampling based on quality setting
        // Note: downsample functions expect 16-bit data
        let (frame_data, effective_width, effective_height) = match self.quality {
            StreamQuality::Preview => downsample_2x2(frame.pixels(), frame.width, frame.height),
            StreamQuality::Fast => downsample_4x4(frame.pixels(), frame.width, frame.height),
            StreamQuality::Full => (frame.pixels().to_vec(), frame.width, frame.height),
        };

        let packet = ObserverFramePacket {
            data: frame_data,
            width: effective_width,
            height: effective_height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
            exposure_ms: frame.exposure_ms,
            roi_x: frame.roi_x,
            roi_y: frame.roi_y,
            temperature_c: frame.temperature_c,
            binning: frame.binning,
        };

        // Non-blocking send - drop frame if channel is full (backpressure)
        if self.tx.try_send(packet).is_err() {
            let dropped = self.frames_dropped.fetch_add(1, Ordering::Relaxed);
            if dropped % 10 == 0 {
                tracing::debug!(
                    device_id = %self.device_id,
                    frames_dropped = dropped + 1,
                    "GrpcStreamObserver dropping frame due to backpressure"
                );
            }
        }
    }

    fn name(&self) -> &str {
        "grpc_stream_observer"
    }
}

// =============================================================================
// Hardware Service Implementation
// =============================================================================

/// Hardware gRPC service implementation
///
/// Provides direct access to hardware devices through the DeviceRegistry.
/// All hardware operations are delegated to the appropriate capability traits.
pub struct HardwareServiceImpl {
    registry: Arc<DeviceRegistry>,
    /// Broadcast sender for parameter changes (enables real-time GUI synchronization)
    param_change_tx: tokio::sync::broadcast::Sender<ParameterChange>,
}

impl HardwareServiceImpl {
    const RPC_TIMEOUT: Duration = Duration::from_secs(15);

    async fn await_with_timeout<F, T>(&self, operation: &str, fut: F) -> Result<T, Status>
    where
        F: Future<Output = Result<T, AnyError>> + Send,
        T: Send,
    {
        match tokio::time::timeout(Self::RPC_TIMEOUT, fut).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(map_hardware_error_to_status(&err.to_string())),
            Err(_) => Err(Status::deadline_exceeded(format!(
                "{} timed out after {:?}",
                operation,
                Self::RPC_TIMEOUT
            ))),
        }
    }

    /// Create a new HardwareService with the given device registry
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        // Create broadcast channel for parameter changes (capacity 256 in-flight messages)
        let (param_change_tx, _) = tokio::sync::broadcast::channel(256);

        // Wire up automatic parameter change notifications (bd-zafg)
        //
        // This monitors all parameters from Parameterized devices and broadcasts changes
        // to gRPC clients via StreamParameterChanges. When hardware drivers call
        // Parameter.set(), those changes automatically propagate to GUI subscribers.
        let registry_clone = registry.clone();
        let tx_clone = param_change_tx.clone();
        tokio::spawn(async move {
            // Give registry time to fully initialize
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Iterate all devices and spawn monitors for parameters
            for device_info in registry_clone.list_devices() {
                let device_id = device_info.id.clone();

                if let Some(parameterized) = registry_clone.get_parameterized(&device_id) {
                    let param_set = parameterized.parameters();
                    // Found a Parameterized device - monitor all its parameters
                    for param_name in param_set.names() {
                        let tx = tx_clone.clone();
                        let dev_id = device_id.clone();
                        let p_name = param_name.to_string();

                        // Monitor Parameter<T> types only for StreamParameterChanges
                        // (configuration/settings that change infrequently)
                        //
                        // Observable<T> types are NOT monitored here - they are high-frequency
                        // sensor readings that should use StreamObservables instead to avoid:
                        // 1. Double traffic (StreamParameterChanges + StreamObservables)
                        // 2. Inefficient string serialization for numeric data
                        //
                        // See bd-ijre for architectural rationale.

                        // f64 parameters (NOT observables)
                        if let Some(p) = param_set.get_typed::<Parameter<f64>>(param_name) {
                            monitor_parameter(p.subscribe(), tx, dev_id, p_name);
                        }
                        // bool parameters
                        else if let Some(p) = param_set.get_typed::<Parameter<bool>>(param_name) {
                            monitor_parameter(p.subscribe(), tx, dev_id, p_name);
                        }
                        // String parameters
                        else if let Some(p) = param_set.get_typed::<Parameter<String>>(param_name)
                        {
                            monitor_parameter(p.subscribe(), tx, dev_id, p_name);
                        }
                        // i64 parameters
                        else if let Some(p) = param_set.get_typed::<Parameter<i64>>(param_name) {
                            monitor_parameter(p.subscribe(), tx, dev_id, p_name);
                        }
                    }
                }
            }
        });

        Self {
            registry,
            param_change_tx,
        }
    }

    /// Create a new HardwareService with an existing parameter change broadcast sender
    /// (useful when sharing the sender across multiple services)
    pub fn with_param_broadcast(
        registry: Arc<DeviceRegistry>,
        param_change_tx: tokio::sync::broadcast::Sender<ParameterChange>,
    ) -> Self {
        Self {
            registry,
            param_change_tx,
        }
    }

    /// Get a clone of the parameter change broadcast sender for external notification
    pub fn param_change_sender(&self) -> tokio::sync::broadcast::Sender<ParameterChange> {
        self.param_change_tx.clone()
    }
}

#[tonic::async_trait]
impl HardwareService for HardwareServiceImpl {
    type SubscribeDeviceStateStream =
        tokio_stream::wrappers::ReceiverStream<Result<DeviceStateUpdate, Status>>;

    // =========================================================================
    // Discovery and Introspection
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "list_devices"))]
    async fn list_devices(
        &self,
        request: Request<ListDevicesRequest>,
    ) -> Result<Response<ListDevicesResponse>, Status> {
        let req = request.into_inner();

        let devices: Vec<DeviceInfo> = if let Some(capability_filter) = req.capability_filter {
            // Filter by capability
            let cap = match capability_filter.to_lowercase().as_str() {
                "movable" => Capability::Movable,
                "readable" => Capability::Readable,
                "triggerable" => Capability::Triggerable,
                "frame_producer" | "frameproducer" => Capability::FrameProducer,
                "exposure_control" | "exposurecontrol" => Capability::ExposureControl,
                _ => {
                    return Err(Status::invalid_argument(format!(
                        "Unknown capability: {}",
                        capability_filter
                    )));
                }
            };

            self.registry
                .devices_with_capability(cap)
                .iter()
                .filter_map(|id| self.registry.get_device_info(id))
                .map(|info| device_info_to_proto(&info))
                .collect()
        } else {
            // Return all devices
            self.registry
                .list_devices()
                .iter()
                .map(device_info_to_proto)
                .collect()
        };

        // Include registration failures for debugging visibility
        let registration_failures: Vec<ProtoRegistrationFailure> = self
            .registry
            .list_registration_failures()
            .into_iter()
            .map(|f| ProtoRegistrationFailure {
                device_id: f.device_id,
                device_name: f.device_name,
                driver_type: f.driver_type,
                error: f.error,
            })
            .collect();

        if !registration_failures.is_empty() {
            tracing::warn!(
                failure_count = registration_failures.len(),
                "ListDevices response includes registration failures"
            );
        }

        Ok(Response::new(ListDevicesResponse {
            devices,
            registration_failures,
        }))
    }

    #[instrument(skip(self, request), fields(method = "get_device_state"))]
    async fn get_device_state(
        &self,
        request: Request<DeviceStateRequest>,
    ) -> Result<Response<DeviceStateResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(device_id = %req.device_id, "GetDeviceState called");

        // Acquire device references without lock
        // This prevents deadlock when hardware operations take time
        let (movable, readable, triggerable, frame_producer, exposure_control, exists) = (
            self.registry.get_movable(&req.device_id),
            self.registry.get_readable(&req.device_id),
            self.registry.get_triggerable(&req.device_id),
            self.registry.get_frame_producer(&req.device_id),
            self.registry.get_exposure_control(&req.device_id),
            self.registry.contains(&req.device_id),
        );

        if !exists {
            return Err(Status::not_found(format!(
                "Device not found: {}",
                req.device_id
            )));
        }

        let mut response = DeviceStateResponse {
            device_id: req.device_id.clone(),
            online: true,
            position: None,
            last_reading: None,
            armed: None,
            streaming: None,
            exposure_ms: None,
        };

        // Now perform async operations WITHOUT holding the lock
        if let Some(movable) = movable {
            match movable.position().await {
                Ok(pos) => {
                    tracing::debug!(device_id = %req.device_id, position = pos, "Got position");
                    response.position = Some(pos);
                }
                Err(e) => {
                    tracing::warn!(device_id = %req.device_id, error = %e, "Position query failed, marking offline");
                    response.online = false;
                }
            }
        }

        if let Some(readable) = readable {
            match readable.read().await {
                Ok(val) => response.last_reading = Some(val),
                Err(_) => {} // Not critical if read fails
            }
        }

        if let Some(triggerable) = triggerable {
            // Convert Result<bool> to Option<bool> at gRPC boundary
            // Err means state couldn't be determined -> None in proto
            response.armed = triggerable.is_armed().await.ok();
        }

        if let Some(frame_producer) = frame_producer {
            // Convert Result<bool> to Option<bool> at gRPC boundary
            response.streaming = frame_producer.is_streaming().await.ok();
        }

        if let Some(exposure_ctrl) = exposure_control
            && let Ok(seconds) = exposure_ctrl.get_exposure().await
        {
            response.exposure_ms = Some(seconds * 1000.0);
        }

        Ok(Response::new(response))
    }

    async fn subscribe_device_state(
        &self,
        request: Request<DeviceStateSubscribeRequest>,
    ) -> Result<Response<Self::SubscribeDeviceStateStream>, Status> {
        let req = request.into_inner();

        // Determine device list and validate device IDs exist
        let device_ids: Vec<String> = if req.device_ids.is_empty() {
            self.registry
                .list_devices()
                .iter()
                .map(|d| d.id.clone())
                .collect()
        } else {
            // Validate all requested device IDs exist
            for device_id in &req.device_ids {
                if !self.registry.contains(device_id) {
                    return Err(Status::not_found(format!(
                        "Device '{}' not found",
                        device_id
                    )));
                }
            }
            req.device_ids.clone()
        };

        if device_ids.is_empty() {
            return Err(Status::not_found("No devices available to subscribe"));
        }

        // Rate limiting interval
        let interval_ms = if req.max_rate_hz > 0 {
            (1000.0 / (req.max_rate_hz as f64)).max(10.0) as u64
        } else {
            200
        };

        let include_snapshot = req.include_snapshot;
        let last_seen_version = req.last_seen_version;
        let registry = Arc::clone(&self.registry);
        let (tx, rx) = tokio::sync::mpsc::channel(32);

        tokio::spawn(async move {
            let mut versions: HashMap<String, u64> = HashMap::new();
            let mut last_payloads: HashMap<String, HashMap<String, String>> = HashMap::new();
            let mut ticker = interval(Duration::from_millis(interval_ms));
            let mut first_tick = true;

            loop {
                ticker.tick().await;
                for device_id in device_ids.iter() {
                    let state = match fetch_device_state(&registry, device_id).await {
                        Ok(s) => s,
                        Err(status) => {
                            let _ = tx.send(Err(status)).await;
                            continue;
                        }
                    };

                    let fields = device_state_to_fields_json(&state);
                    let prev = last_payloads.get(device_id);
                    let changed = match prev {
                        None => true,
                        Some(p) => p != &fields,
                    };

                    let current_version = versions
                        .get(device_id)
                        .cloned()
                        .unwrap_or(last_seen_version);
                    let next_version = current_version.saturating_add(1);
                    let is_snapshot =
                        (include_snapshot && first_tick) || (current_version < last_seen_version);

                    if is_snapshot || changed {
                        let update = DeviceStateUpdate {
                            device_id: device_id.clone(),
                            timestamp_ns: now_ns(),
                            version: next_version,
                            is_snapshot,
                            fields_json: fields.clone(),
                        };
                        if tx.send(Ok(update)).await.is_err() {
                            return;
                        }
                        versions.insert(device_id.clone(), next_version);
                        last_payloads.insert(device_id.clone(), fields);
                    }
                }
                first_tick = false;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    // =========================================================================
    // Motion Control
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "move_absolute"))]
    async fn move_absolute(
        &self,
        request: Request<MoveRequest>,
    ) -> Result<Response<MoveResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let movable = self.registry.get_movable(&req.device_id);

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        self.await_with_timeout("move_abs", movable.move_abs(req.value))
            .await?;

        let (final_position, settled) = if req.wait_for_completion.unwrap_or(false) {
            if let Some(timeout_ms) = req.timeout_ms {
                match tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    movable.wait_settled(),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        let pos = movable.position().await.map_err(|e| {
                            tracing::error!(device_id = %req.device_id, error = %e, "Failed to verify position after move");
                            Status::unavailable(format!("Move completed but position verification failed: {}", e))
                        })?;
                        (pos, Some(true))
                    }
                    Ok(Err(e)) => {
                        return Err(map_hardware_error_to_status(&e.to_string()));
                    }
                    Err(_) => {
                        // On timeout, try to get position but use NaN if read fails (don't mislead with target)
                        let pos = movable.position().await.unwrap_or(f64::NAN);
                        return Err(Status::deadline_exceeded(format!(
                            "Motion did not complete within {} ms, current position: {}",
                            req.timeout_ms.unwrap_or(0),
                            pos
                        )));
                    }
                }
            } else {
                self.await_with_timeout("wait_settled", movable.wait_settled())
                    .await?;
                let pos = movable.position().await.map_err(|e| {
                    tracing::error!(device_id = %req.device_id, error = %e, "Failed to verify position after move");
                    Status::unavailable(format!("Move completed but position verification failed: {}", e))
                })?;
                (pos, Some(true))
            }
        } else {
            let pos = movable.position().await.map_err(|e| {
                tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after move");
                Status::unavailable(format!("Move initiated but position read failed: {}", e))
            })?;
            (pos, None)
        };

        Ok(Response::new(MoveResponse {
            success: true,
            error_message: String::new(),
            final_position,
            settled,
        }))
    }

    #[instrument(skip(self, request), fields(method = "move_relative"))]
    async fn move_relative(
        &self,
        request: Request<MoveRequest>,
    ) -> Result<Response<MoveResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let movable = self.registry.get_movable(&req.device_id);

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        self.await_with_timeout("move_rel", movable.move_rel(req.value))
            .await?;

        let (final_position, settled) = if req.wait_for_completion.unwrap_or(false) {
            if let Some(timeout_ms) = req.timeout_ms {
                match tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    movable.wait_settled(),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        let pos = movable.position().await.map_err(|e| {
                            tracing::error!(device_id = %req.device_id, error = %e, "Failed to verify position after relative move");
                            Status::unavailable(format!("Move completed but position verification failed: {}", e))
                        })?;
                        (pos, Some(true))
                    }
                    Ok(Err(e)) => {
                        return Err(map_hardware_error_to_status(&e.to_string()));
                    }
                    Err(_) => {
                        // On timeout, try to get position but use NaN if read fails (don't mislead with 0.0)
                        let pos = movable.position().await.unwrap_or(f64::NAN);
                        return Err(Status::deadline_exceeded(format!(
                            "Motion did not complete within {} ms, current position: {}",
                            req.timeout_ms.unwrap_or(0),
                            pos
                        )));
                    }
                }
            } else {
                self.await_with_timeout("wait_settled", movable.wait_settled())
                    .await?;
                let pos = movable.position().await.map_err(|e| {
                    tracing::error!(device_id = %req.device_id, error = %e, "Failed to verify position after relative move");
                    Status::unavailable(format!("Move completed but position verification failed: {}", e))
                })?;
                (pos, Some(true))
            }
        } else {
            let pos = movable.position().await.map_err(|e| {
                tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after relative move");
                Status::unavailable(format!("Move initiated but position read failed: {}", e))
            })?;
            (pos, None)
        };

        Ok(Response::new(MoveResponse {
            success: true,
            error_message: String::new(),
            final_position,
            settled,
        }))
    }

    #[instrument(skip(self, request), fields(method = "stop_motion"))]
    async fn stop_motion(
        &self,
        request: Request<StopMotionRequest>,
    ) -> Result<Response<StopMotionResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let movable = self.registry.get_movable(&req.device_id);

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        self.await_with_timeout("stop_motion", movable.stop())
            .await?;

        let position = movable.position().await.map_err(|e| {
            tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after stop");
            Status::unavailable(format!("Stop completed but position read failed: {}", e))
        })?;
        Ok(Response::new(StopMotionResponse {
            success: true,
            stopped_position: position,
        }))
    }

    async fn wait_settled(
        &self,
        request: Request<WaitSettledRequest>,
    ) -> Result<Response<WaitSettledResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let movable = self.registry.get_movable(&req.device_id);

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        if let Some(timeout_ms) = req.timeout_ms {
            match tokio::time::timeout(
                Duration::from_millis(timeout_ms as u64),
                movable.wait_settled(),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(map_hardware_error_to_status(&e.to_string())),
                Err(_) => {
                    return Err(Status::deadline_exceeded(format!(
                        "Wait settled operation timed out for device '{}'",
                        req.device_id
                    )));
                }
            }
        } else {
            self.await_with_timeout("wait_settled", movable.wait_settled())
                .await?;
        }

        let position = movable.position().await.map_err(|e| {
            tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after wait_settled");
            Status::unavailable(format!("Wait settled completed but position read failed: {}", e))
        })?;
        Ok(Response::new(WaitSettledResponse {
            success: true,
            settled: true,
            position,
        }))
    }

    type StreamPositionStream =
        tokio_stream::wrappers::ReceiverStream<Result<PositionUpdate, Status>>;

    async fn stream_position(
        &self,
        request: Request<StreamPositionRequest>,
    ) -> Result<Response<Self::StreamPositionStream>, Status> {
        let req = request.into_inner();
        let registry = self.registry.clone();
        let device_id = req.device_id.clone();
        let rate_hz = req.rate_hz.max(1); // Minimum 1 Hz

        // Verify device exists and is movable
        if self.registry.get_movable(&device_id).is_none() {
            return Err(Status::not_found(format!(
                "Device '{}' not found or not movable",
                device_id
            )));
        }

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs_f64(1.0 / rate_hz as f64);
            let mut ticker = tokio::time::interval(interval);
            let mut last_position = f64::NAN;

            loop {
                ticker.tick().await;

                // Get movable directly from registry
                let movable = registry.get_movable(&device_id);

                if let Some(movable) = movable {
                    let position = movable.position().await.unwrap_or(f64::NAN);
                    let is_moving = (position - last_position).abs() > 0.0001;
                    last_position = position;

                    let update = PositionUpdate {
                        device_id: device_id.clone(),
                        position,
                        timestamp_ns: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64,
                        is_moving,
                    };

                    if tx.send(Ok(update)).await.is_err() {
                        break; // Client disconnected
                    }
                } else {
                    break; // Device removed
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    // =========================================================================
    // Scalar Readout
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "read_value"))]
    async fn read_value(
        &self,
        request: Request<ReadValueRequest>,
    ) -> Result<Response<ReadValueResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and metadata without lock before awaiting hardware
        let readable = self.registry.get_readable(&req.device_id);
        let units = self
            .registry
            .get_device_info(&req.device_id)
            .and_then(|info| info.metadata.measurement_units.clone())
            .unwrap_or_default();

        let readable = readable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not readable",
                req.device_id
            ))
        })?;

        let value = self
            .await_with_timeout("read_value", readable.read())
            .await?;

        Ok(Response::new(ReadValueResponse {
            success: true,
            error_message: String::new(),
            value,
            units,
            timestamp_ns: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
        }))
    }

    type StreamValuesStream = tokio_stream::wrappers::ReceiverStream<Result<ValueUpdate, Status>>;

    async fn stream_values(
        &self,
        request: Request<StreamValuesRequest>,
    ) -> Result<Response<Self::StreamValuesStream>, Status> {
        let req = request.into_inner();
        let registry = self.registry.clone();
        let device_id = req.device_id.clone();
        let rate_hz = req.rate_hz.max(1);

        // Verify device exists and is readable
        if self.registry.get_readable(&device_id).is_none() {
            return Err(Status::not_found(format!(
                "Device '{}' not found or not readable",
                device_id
            )));
        }

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs_f64(1.0 / rate_hz as f64);
            let mut ticker = tokio::time::interval(interval);

            loop {
                ticker.tick().await;

                // Get readable and metadata directly from registry
                let readable = registry.get_readable(&device_id);
                let units = registry
                    .get_device_info(&device_id)
                    .and_then(|info| info.metadata.measurement_units.clone())
                    .unwrap_or_default();

                if let Some(readable) = readable {
                    if let Ok(value) = readable.read().await {
                        let update = ValueUpdate {
                            device_id: device_id.clone(),
                            value,
                            units,
                            timestamp_ns: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos() as u64,
                        };

                        if tx.send(Ok(update)).await.is_err() {
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    // =========================================================================
    // Trigger Control
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "arm"))]
    async fn arm(&self, request: Request<ArmRequest>) -> Result<Response<ArmResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let triggerable = self.registry.get_triggerable(&req.device_id);

        let triggerable = triggerable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not triggerable",
                req.device_id
            ))
        })?;

        match triggerable.arm().await {
            Ok(_) => Ok(Response::new(ArmResponse {
                success: true,
                error_message: String::new(),
                armed: true,
            })),
            Err(e) => {
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }

    #[instrument(skip(self, request), fields(method = "trigger"))]
    async fn trigger(
        &self,
        request: Request<TriggerRequest>,
    ) -> Result<Response<TriggerResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let triggerable = self.registry.get_triggerable(&req.device_id);

        let triggerable = triggerable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not triggerable",
                req.device_id
            ))
        })?;

        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        match triggerable.trigger().await {
            Ok(_) => Ok(Response::new(TriggerResponse {
                success: true,
                error_message: String::new(),
                trigger_timestamp_ns: timestamp_ns,
            })),
            Err(e) => {
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }

    // =========================================================================
    // Exposure Control
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "set_exposure"))]
    async fn set_exposure(
        &self,
        request: Request<SetExposureRequest>,
    ) -> Result<Response<SetExposureResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let exposure_ctrl = self.registry.get_exposure_control(&req.device_id);

        let exposure_ctrl = exposure_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no exposure control",
                req.device_id
            ))
        })?;

        // Convert ms to seconds for the trait API
        let exposure_seconds = req.exposure_ms / 1000.0;

        match exposure_ctrl.set_exposure(exposure_seconds).await {
            Ok(_) => {
                // Convert seconds back to ms for response
                let actual_seconds = exposure_ctrl
                    .get_exposure()
                    .await
                    .unwrap_or(exposure_seconds);
                Ok(Response::new(SetExposureResponse {
                    success: true,
                    error_message: String::new(),
                    actual_exposure_ms: actual_seconds * 1000.0,
                }))
            }
            Err(e) => {
                let err_msg = e.to_string();
                // Check for out-of-range errors
                if err_msg.contains("out of range")
                    || err_msg.contains("bounds")
                    || err_msg.contains("invalid")
                {
                    Err(Status::invalid_argument(format!(
                        "Invalid exposure value: {}",
                        req.exposure_ms
                    )))
                } else {
                    let status = map_hardware_error_to_status(&err_msg);
                    Err(status)
                }
            }
        }
    }

    async fn get_exposure(
        &self,
        request: Request<GetExposureRequest>,
    ) -> Result<Response<GetExposureResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let exposure_ctrl = self.registry.get_exposure_control(&req.device_id);

        let exposure_ctrl = exposure_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no exposure control",
                req.device_id
            ))
        })?;

        // Convert seconds to ms for response
        match exposure_ctrl.get_exposure().await {
            Ok(seconds) => Ok(Response::new(GetExposureResponse {
                exposure_ms: seconds * 1000.0,
            })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to get exposure: {}",
                e
            ))),
        }
    }

    // =========================================================================
    // Laser Control (bd-pwjo)
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "set_shutter"))]
    async fn set_shutter(
        &self,
        request: Request<SetShutterRequest>,
    ) -> Result<Response<SetShutterResponse>, Status> {
        let req = request.into_inner();

        let shutter_ctrl = self.registry.get_shutter_control(&req.device_id);

        let shutter_ctrl = shutter_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no shutter control",
                req.device_id
            ))
        })?;

        let open = req.open;
        match if open {
            shutter_ctrl.open_shutter().await
        } else {
            shutter_ctrl.close_shutter().await
        } {
            Ok(()) => Ok(Response::new(SetShutterResponse {
                success: true,
                error_message: String::new(),
                is_open: open,
            })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to set shutter: {}",
                e
            ))),
        }
    }

    async fn get_shutter(
        &self,
        request: Request<GetShutterRequest>,
    ) -> Result<Response<GetShutterResponse>, Status> {
        let req = request.into_inner();

        let shutter_ctrl = self.registry.get_shutter_control(&req.device_id);

        let shutter_ctrl = shutter_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no shutter control",
                req.device_id
            ))
        })?;

        match shutter_ctrl.is_shutter_open().await {
            Ok(is_open) => Ok(Response::new(GetShutterResponse { is_open })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to get shutter state: {}",
                e
            ))),
        }
    }

    #[instrument(skip(self, request), fields(method = "set_wavelength"))]
    async fn set_wavelength(
        &self,
        request: Request<SetWavelengthRequest>,
    ) -> Result<Response<SetWavelengthResponse>, Status> {
        let req = request.into_inner();

        let wavelength_ctrl = self.registry.get_wavelength_tunable(&req.device_id);

        let wavelength_ctrl = wavelength_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no wavelength control",
                req.device_id
            ))
        })?;

        let requested_nm = req.wavelength_nm;
        match wavelength_ctrl.set_wavelength(requested_nm).await {
            Ok(()) => Ok(Response::new(SetWavelengthResponse {
                success: true,
                error_message: String::new(),
                actual_wavelength_nm: requested_nm,
            })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to set wavelength: {}",
                e
            ))),
        }
    }

    async fn get_wavelength(
        &self,
        request: Request<GetWavelengthRequest>,
    ) -> Result<Response<GetWavelengthResponse>, Status> {
        let req = request.into_inner();

        let wavelength_ctrl = self.registry.get_wavelength_tunable(&req.device_id);

        let wavelength_ctrl = wavelength_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no wavelength control",
                req.device_id
            ))
        })?;

        match wavelength_ctrl.get_wavelength().await {
            Ok(nm) => Ok(Response::new(GetWavelengthResponse { wavelength_nm: nm })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to get wavelength: {}",
                e
            ))),
        }
    }

    #[instrument(skip(self, request), fields(method = "set_emission"))]
    async fn set_emission(
        &self,
        request: Request<SetEmissionRequest>,
    ) -> Result<Response<SetEmissionResponse>, Status> {
        let req = request.into_inner();

        let emission_ctrl = self.registry.get_emission_control(&req.device_id);

        let emission_ctrl = emission_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no emission control",
                req.device_id
            ))
        })?;

        let enabled = req.enabled;
        match if enabled {
            emission_ctrl.enable_emission().await
        } else {
            emission_ctrl.disable_emission().await
        } {
            Ok(()) => Ok(Response::new(SetEmissionResponse {
                success: true,
                error_message: String::new(),
                is_enabled: enabled,
            })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to set emission: {}",
                e
            ))),
        }
    }

    async fn get_emission(
        &self,
        request: Request<GetEmissionRequest>,
    ) -> Result<Response<GetEmissionResponse>, Status> {
        let req = request.into_inner();

        let emission_ctrl = self.registry.get_emission_control(&req.device_id);

        let emission_ctrl = emission_ctrl.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or has no emission control",
                req.device_id
            ))
        })?;

        match emission_ctrl.is_emission_enabled().await {
            Ok(is_enabled) => Ok(Response::new(GetEmissionResponse { is_enabled })),
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to get emission state: {}",
                e
            ))),
        }
    }

    // =========================================================================
    // Frame Streaming
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "start_stream"))]
    async fn start_stream(
        &self,
        request: Request<StartStreamRequest>,
    ) -> Result<Response<StartStreamResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc without lock before awaiting hardware
        let frame_producer = self.registry.get_frame_producer(&req.device_id);

        let frame_producer = frame_producer.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not a frame producer",
                req.device_id
            ))
        })?;

        // Use frame_count from request (0 or None = continuous)
        let frame_limit = req.frame_count.filter(|&n| n > 0);

        match frame_producer.start_stream_finite(frame_limit).await {
            Ok(_) => Ok(Response::new(StartStreamResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => {
                let err_msg = e.to_string();
                // Idempotent: treat "already streaming" as success
                if err_msg.to_lowercase().contains("already streaming") {
                    tracing::info!(device_id = %req.device_id, "Device already streaming (idempotent success)");
                    Ok(Response::new(StartStreamResponse {
                        success: true,
                        error_message: "Already streaming".to_string(),
                    }))
                } else {
                    let status = map_hardware_error_to_status(&err_msg);
                    Err(status)
                }
            }
        }
    }

    #[instrument(skip(self, request), fields(method = "stop_stream"))]
    async fn stop_stream(
        &self,
        request: Request<StopStreamRequest>,
    ) -> Result<Response<StopStreamResponse>, Status> {
        let req = request.into_inner();
        eprintln!(
            "[GRPC DEBUG] stop_stream called for device: {}",
            req.device_id
        );

        // Extract Arc without lock before awaiting hardware
        let frame_producer = self.registry.get_frame_producer(&req.device_id);

        let frame_producer = frame_producer.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not a frame producer",
                req.device_id
            ))
        })?;

        match frame_producer.stop_stream().await {
            Ok(_) => {
                // Get frame count from device
                let frames_captured = frame_producer.frame_count();
                Ok(Response::new(StopStreamResponse {
                    success: true,
                    frames_captured,
                }))
            }
            Err(e) => Err(map_hardware_error_to_status(&format!(
                "Failed to stop stream: {}",
                e
            ))),
        }
    }

    type StreamFramesStream = ReceiverStream<Result<FrameData, Status>>;

    /// Stream frames from a FrameProducer device to GUI clients (bd-0dax.6.3).
    ///
    /// Uses the tap-based observer pattern (`register_observer`) to receive frames.
    /// This is more efficient than the deprecated `subscribe_frames()` broadcast
    /// approach because:
    /// - Observers receive borrowed `FrameView` references (zero-copy from driver)
    /// - Downsampling happens in the observer callback (before any channel send)
    /// - Backpressure is handled locally in the observer
    ///
    /// Supports optional rate limiting via max_fps.
    async fn stream_frames(
        &self,
        request: Request<StreamFramesRequest>,
    ) -> Result<Response<Self::StreamFramesStream>, Status> {
        let req = request.into_inner();
        let device_id = req.device_id.clone();
        let max_fps = req.max_fps;
        let quality = req.quality();

        // Get frame producer
        let frame_producer = self.registry.get_frame_producer(&device_id);

        let frame_producer = frame_producer.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not a frame producer",
                device_id
            ))
        })?;

        // Check if device supports observers (bd-0dax.6.3)
        if !frame_producer.supports_observers() {
            return Err(Status::unavailable(format!(
                "Device '{}' does not support frame observers. \
                 The driver must implement register_observer() for tap-based streaming.",
                device_id
            )));
        }

        // Channel capacity constants
        // Observer channel: bounded to handle backpressure in on_frame()
        const OBSERVER_CHANNEL_CAPACITY: usize = 16;
        // gRPC channel: buffer for network jitter (bd-7rk0)
        const GRPC_CHANNEL_CAPACITY: usize = 8;
        const GRPC_SKIP_THRESHOLD: usize = 6; // 75% full triggers frame skipping

        // Create channel from observer to forwarding task
        let (observer_tx, mut observer_rx) =
            tokio::sync::mpsc::channel::<ObserverFramePacket>(OBSERVER_CHANNEL_CAPACITY);

        // Create gRPC stream observer
        let observer = GrpcStreamObserver::new(observer_tx, quality, device_id.clone());

        // Register the observer with the frame producer
        let observer_handle = frame_producer
            .register_observer(Box::new(observer))
            .await
            .map_err(|e| {
                Status::internal(format!(
                    "Failed to register frame observer for device '{}': {}",
                    device_id, e
                ))
            })?;

        tracing::info!(
            device_id = %device_id,
            observer_handle = observer_handle.id(),
            max_fps = max_fps,
            quality = ?quality,
            "Registered gRPC stream observer"
        );

        // Create output channel for gRPC stream
        let (grpc_tx, grpc_rx) = tokio::sync::mpsc::channel(GRPC_CHANNEL_CAPACITY);

        // Calculate minimum interval between frames for rate limiting
        let min_interval = if max_fps > 0 {
            Some(Duration::from_secs_f64(1.0 / max_fps as f64))
        } else {
            None
        };

        // Spawn task to forward frames from observer channel to gRPC stream
        let device_id_clone = device_id.clone();
        let frame_producer_clone = frame_producer.clone();
        tokio::spawn(async move {
            // Initialize to allow first frame through immediately
            let mut last_frame_time = match min_interval {
                Some(interval) => std::time::Instant::now() - interval,
                None => std::time::Instant::now(),
            };
            let mut frames_sent = 0u64;
            let mut frames_dropped = 0u64;
            let mut fps_window: VecDeque<std::time::Instant> = VecDeque::new();
            let mut avg_latency_ms = 0.0f64;
            let mut latency_samples = 0u64;

            tracing::info!(
                device_id = %device_id_clone,
                max_fps = max_fps,
                quality = ?quality,
                observer_channel_capacity = OBSERVER_CHANNEL_CAPACITY,
                grpc_channel_capacity = GRPC_CHANNEL_CAPACITY,
                "Starting tap-based frame stream forwarding task"
            );

            let exit_reason: &str;

            loop {
                match observer_rx.recv().await {
                    Some(packet) => {
                        // Log early frames for debugging
                        if frames_sent < 10 {
                            tracing::info!(
                                device_id = %device_id_clone,
                                frame_number = packet.frame_number,
                                bytes = packet.data.len(),
                                width = packet.width,
                                height = packet.height,
                                "Received frame from observer (early frame debug)"
                            );
                        }

                        // Rate limiting: skip frame if too soon
                        if let Some(interval) = min_interval {
                            let elapsed = last_frame_time.elapsed();
                            if elapsed < interval {
                                frames_dropped = frames_dropped.saturating_add(1);
                                continue;
                            }
                        }
                        last_frame_time = std::time::Instant::now();

                        // Backpressure handling: skip frames if gRPC channel is nearly full
                        let queue_len = GRPC_CHANNEL_CAPACITY - grpc_tx.capacity();
                        if queue_len >= GRPC_SKIP_THRESHOLD {
                            frames_dropped = frames_dropped.saturating_add(1);
                            if frames_dropped % 10 == 1 {
                                tracing::debug!(
                                    device_id = %device_id_clone,
                                    queue_len,
                                    threshold = GRPC_SKIP_THRESHOLD,
                                    "Skipping frame due to gRPC backpressure"
                                );
                            }
                            continue;
                        }

                        // Validate frame dimensions (bd-7rk0)
                        let bytes_per_pixel = (packet.bit_depth as usize).div_ceil(8);
                        let expected_size = (packet.width as usize)
                            .saturating_mul(packet.height as usize)
                            .saturating_mul(bytes_per_pixel);
                        if packet.data.len() != expected_size {
                            tracing::warn!(
                                device_id = %device_id_clone,
                                width = packet.width,
                                height = packet.height,
                                bit_depth = packet.bit_depth,
                                actual_size = packet.data.len(),
                                expected_size = expected_size,
                                "Frame data size mismatch after downsampling, skipping"
                            );
                            frames_dropped = frames_dropped.saturating_add(1);
                            continue;
                        }

                        // Update FPS tracking
                        let now_instant = std::time::Instant::now();
                        fps_window.push_back(now_instant);
                        while let Some(front) = fps_window.front() {
                            if now_instant.duration_since(*front) > Duration::from_secs(1) {
                                fps_window.pop_front();
                            } else {
                                break;
                            }
                        }
                        let current_fps = fps_window.len() as f64;

                        // Update latency tracking
                        if packet.timestamp_ns > 0 {
                            let latency_ms =
                                now_ns().saturating_sub(packet.timestamp_ns) as f64 / 1_000_000.0;
                            latency_samples = latency_samples.saturating_add(1);
                            avg_latency_ms +=
                                (latency_ms - avg_latency_ms) / latency_samples as f64;
                        }

                        frames_sent = frames_sent.saturating_add(1);
                        let metrics = StreamingMetrics {
                            current_fps,
                            frames_sent,
                            frames_dropped,
                            avg_latency_ms,
                        };

                        // Build FrameData proto and apply compression in blocking task
                        let device_id_for_frame = device_id_clone.clone();
                        let processing_result = tokio::task::spawn_blocking(move || {
                            let mut frame_data = FrameData {
                                device_id: device_id_for_frame,
                                width: packet.width,
                                height: packet.height,
                                bit_depth: packet.bit_depth,
                                data: packet.data,
                                frame_number: packet.frame_number,
                                timestamp_ns: packet.timestamp_ns,
                                exposure_ms: packet.exposure_ms,
                                roi_x: packet.roi_x,
                                roi_y: packet.roi_y,
                                temperature_c: packet.temperature_c,
                                gain_mode: None, // FrameView doesn't include these
                                readout_speed: None,
                                trigger_mode: None,
                                binning_x: packet.binning.map(|(x, _)| x as u32),
                                binning_y: packet.binning.map(|(_, y)| y as u32),
                                metadata: HashMap::new(),
                                metrics: Some(metrics),
                                compression: CompressionType::CompressionNone as i32,
                                uncompressed_size: 0,
                            };

                            // Apply LZ4 compression (bd-7rk0)
                            let uncompressed_size = frame_data.data.len();
                            crate::grpc::compression::compress_frame(&mut frame_data);
                            let compressed_size = frame_data.data.len();

                            (frame_data, uncompressed_size, compressed_size)
                        })
                        .await;

                        let (frame_data, uncompressed_size, compressed_size) =
                            match processing_result {
                                Ok(result) => result,
                                Err(e) => {
                                    tracing::error!(
                                        device_id = %device_id_clone,
                                        error = %e,
                                        "Frame compression task panicked or was cancelled"
                                    );
                                    frames_dropped = frames_dropped.saturating_add(1);
                                    continue;
                                }
                            };

                        // Log early frame sends
                        if frames_sent <= 10 {
                            tracing::info!(
                                device_id = %device_id_clone,
                                frame = frames_sent,
                                frame_number = frame_data.frame_number,
                                bytes = frame_data.data.len(),
                                queue_capacity = grpc_tx.capacity(),
                                "About to send frame to gRPC client (early frame debug)"
                            );
                        }

                        // Send to gRPC client
                        if grpc_tx.send(Ok(frame_data)).await.is_err() {
                            eprintln!(
                                "[GRPC DEBUG] Client disconnected after {} frames - gRPC send failed",
                                frames_sent
                            );
                            tracing::warn!(
                                device_id = %device_id_clone,
                                frames_sent = frames_sent,
                                "Client disconnected from frame stream - gRPC send failed"
                            );

                            // Unregister observer on disconnect (bd-0dax.6.3)
                            if let Err(e) = frame_producer_clone
                                .unregister_observer(observer_handle)
                                .await
                            {
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    observer_handle = observer_handle.id(),
                                    error = %e,
                                    "Failed to unregister observer on client disconnect"
                                );
                            } else {
                                tracing::info!(
                                    device_id = %device_id_clone,
                                    observer_handle = observer_handle.id(),
                                    "Unregistered observer after client disconnect"
                                );
                            }

                            exit_reason = "client_disconnected";
                            break;
                        }

                        // Log early frame sends success
                        if frames_sent <= 10 {
                            tracing::info!(
                                device_id = %device_id_clone,
                                frame = frames_sent,
                                "Successfully sent frame to gRPC client (early frame debug)"
                            );
                        }

                        // Log compression stats periodically
                        if frames_sent > 10 && frames_sent.is_multiple_of(30) {
                            let ratio = if compressed_size > 0 {
                                uncompressed_size as f64 / compressed_size as f64
                            } else {
                                1.0
                            };
                            tracing::debug!(
                                device_id = %device_id_clone,
                                frames = frames_sent,
                                uncompressed_kb = uncompressed_size / 1024,
                                compressed_kb = compressed_size / 1024,
                                compression_ratio = format!("{:.1}x", ratio),
                                "Sent frame to client (LZ4 compressed)"
                            );
                        }
                    }
                    None => {
                        // Observer channel closed - producer stopped or observer was dropped
                        eprintln!(
                            "[GRPC DEBUG] Observer channel closed for {} after {} frames - producer stopped",
                            device_id_clone, frames_sent
                        );
                        tracing::info!(
                            device_id = %device_id_clone,
                            frames_sent = frames_sent,
                            "Observer channel closed - producer stopped streaming"
                        );

                        // Clean up observer registration
                        if let Err(e) = frame_producer_clone
                            .unregister_observer(observer_handle)
                            .await
                        {
                            tracing::debug!(
                                device_id = %device_id_clone,
                                observer_handle = observer_handle.id(),
                                error = %e,
                                "Failed to unregister observer (may already be unregistered)"
                            );
                        }

                        exit_reason = "observer_channel_closed";
                        break;
                    }
                }
            }

            // Final summary log
            tracing::info!(
                device_id = %device_id_clone,
                exit_reason = exit_reason,
                frames_sent = frames_sent,
                frames_dropped = frames_dropped,
                "Tap-based frame stream forwarding task ended"
            );
        });

        Ok(Response::new(ReceiverStream::new(grpc_rx)))
    }

    // =========================================================================
    // Device Lifecycle (Stage/Unstage - Bluesky pattern)
    // =========================================================================

    /// Stage a device for acquisition (Bluesky-style lifecycle).
    ///
    /// Staging prepares a device before a scan or acquisition sequence.
    /// This is called once at the beginning of a scan for each device involved.
    ///
    /// If the device implements Stageable, calls device.stage(). Otherwise,
    /// staging is a no-op that validates the device exists.
    #[instrument(skip(self, request), fields(method = "stage_device"))]
    async fn stage_device(
        &self,
        request: Request<StageDeviceRequest>,
    ) -> Result<Response<StageDeviceResponse>, Status> {
        let req = request.into_inner();
        let stageable = self.registry.get_stageable(&req.device_id);
        let exists = self.registry.contains(&req.device_id);

        // Verify device exists
        if !exists {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        // If device implements Stageable, call stage()
        if let Some(stageable) = stageable {
            stageable.stage().await.map_err(|e| {
                map_hardware_error_to_status(&format!(
                    "Failed to stage device '{}': {}",
                    req.device_id, e
                ))
            })?;
            tracing::info!("Staged device '{}' successfully", req.device_id);
        } else {
            // No-op for devices that don't implement Stageable
            tracing::debug!(
                "Staged device '{}' (no Stageable impl, no-op)",
                req.device_id
            );
        }

        Ok(Response::new(StageDeviceResponse {
            success: true,
            error_message: String::new(),
            staged: true,
        }))
    }

    /// Unstage a device after acquisition (Bluesky-style lifecycle).
    ///
    /// Unstaging cleans up a device after a scan or acquisition sequence.
    /// This is called once at the end of a scan for each device involved.
    ///
    /// If the device implements Stageable, calls device.unstage(). Otherwise,
    /// unstaging is a no-op that validates the device exists.
    #[instrument(skip(self, request), fields(method = "unstage_device"))]
    async fn unstage_device(
        &self,
        request: Request<UnstageDeviceRequest>,
    ) -> Result<Response<UnstageDeviceResponse>, Status> {
        let req = request.into_inner();
        let stageable = self.registry.get_stageable(&req.device_id);
        let exists = self.registry.contains(&req.device_id);

        // Verify device exists
        if !exists {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        // If device implements Stageable, call unstage()
        if let Some(stageable) = stageable {
            stageable.unstage().await.map_err(|e| {
                map_hardware_error_to_status(&format!(
                    "Failed to unstage device '{}': {}",
                    req.device_id, e
                ))
            })?;
            tracing::info!("Unstaged device '{}' successfully", req.device_id);
        } else {
            // No-op for devices that don't implement Stageable
            tracing::debug!(
                "Unstaged device '{}' (no Stageable impl, no-op)",
                req.device_id
            );
        }

        Ok(Response::new(UnstageDeviceResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    // =========================================================================
    // Passthrough Commands (escape hatch for device-specific features)
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "execute_device_command"))]
    async fn execute_device_command(
        &self,
        request: Request<DeviceCommandRequest>,
    ) -> Result<Response<DeviceCommandResponse>, Status> {
        let req = request.into_inner();

        // Try the new generic Commandable interface first
        if let Some(device) = self.registry.get_commandable(&req.device_id) {
            // Parse arguments as JSON
            const MAX_ARGS_LEN: usize = 64 * 1024; // 64KB
            if req.args.len() > MAX_ARGS_LEN {
                return Err(Status::invalid_argument(format!(
                    "Arguments too large: {} bytes (max {})",
                    req.args.len(),
                    MAX_ARGS_LEN
                )));
            }

            let args = if req.args.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&req.args).map_err(|e| {
                    Status::invalid_argument(format!("Failed to parse command arguments: {}", e))
                })?
            };

            let result = device
                .execute_command(&req.command, args)
                .await
                .map_err(|e| {
                    map_hardware_error_to_status(&format!("Command execution failed: {}", e))
                })?;

            return Ok(Response::new(DeviceCommandResponse {
                success: true,
                error_message: String::new(),
                results: result.to_string(),
            }));
        }

        // Device doesn't implement Commandable trait
        Err(Status::unimplemented(format!(
            "Device '{}' does not support commands. Use capability-specific endpoints \
             (e.g., SetEmission for emission control) or implement Commandable trait.",
            req.device_id
        )))
    }

    // =========================================================================
    // Observable Parameters (QCodes/ScopeFoundry pattern)
    // =========================================================================

    async fn list_parameters(
        &self,
        request: Request<ListParametersRequest>,
    ) -> Result<Response<ListParametersResponse>, Status> {
        let req = request.into_inner();

        // Check if device exists
        if !self.registry.contains(&req.device_id) {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        let mut parameters = Vec::new();

        // 1. Get V5 parameters from Parameterized devices
        if let Some(parameterized) = self.registry.get_parameterized(&req.device_id) {
            let param_set = parameterized.parameters();
            for param_name in param_set.names() {
                if let Some(param) = param_set.get(param_name) {
                    let metadata = param.metadata();

                    // Use introspectable dtype from metadata if available,
                    // otherwise infer from current value (best-effort fallback)
                    let dtype = if !metadata.dtype.is_empty() {
                        metadata.dtype.clone()
                    } else {
                        // Fallback: infer dtype from current value
                        match param.get_json() {
                            Ok(json) => match json {
                                serde_json::Value::Bool(_) => "bool".to_string(),
                                serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => {
                                    "int".to_string()
                                }
                                serde_json::Value::Number(_) => "float".to_string(),
                                serde_json::Value::String(_) => "string".to_string(),
                                serde_json::Value::Array(_) => "array".to_string(),
                                serde_json::Value::Object(_) => "object".to_string(),
                                serde_json::Value::Null => "unknown".to_string(),
                            },
                            Err(_) => "unknown".to_string(),
                        }
                    };

                    parameters.push(ParameterDescriptor {
                        device_id: req.device_id.clone(),
                        name: metadata.name.clone(),
                        description: metadata.description.clone().unwrap_or_default(),
                        dtype,
                        units: metadata.units.clone().unwrap_or_default(),
                        readable: true,
                        writable: !metadata.read_only,
                        min_value: metadata.min_value, // Phase 2 (bd-cdh5.2): introspectable from metadata
                        max_value: metadata.max_value, // Phase 2 (bd-cdh5.2): introspectable from metadata
                        enum_values: metadata.enum_values.clone(), // Phase 2 (bd-cdh5.2): introspectable from metadata
                    });
                }
            }
        }

        // 2. Get settable parameters for plugin devices (V4/Plugin pattern)
        // 2. Get settable parameters for plugin devices (V4/Plugin pattern)
        // NOTE: Plugins now use Parameterized trait (V5) so they are handled by block 1 above.
        // The legacy get_settable_parameters method has been removed.

        Ok(Response::new(ListParametersResponse { parameters }))
    }

    async fn get_parameter(
        &self,
        request: Request<GetParameterRequest>,
    ) -> Result<Response<ParameterValue>, Status> {
        let req = request.into_inner();

        // Try legacy Settable trait first (backwards compatibility)
        if let Some(settable) = self.registry.get_settable(&req.device_id) {
            // Get the parameter value
            let value = settable.get_value(&req.parameter_name).await.map_err(|e| {
                map_hardware_error_to_status(&format!("Failed to get parameter: {}", e))
            })?;

            // Get timestamp
            let timestamp_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);

            return Ok(Response::new(ParameterValue {
                device_id: req.device_id,
                name: req.parameter_name,
                value: value.to_string(),
                units: String::new(), // Would need parameter metadata
                timestamp_ns,
            }));
        }

        // New path - use Parameterized trait
        if let Some(parameterized) = self.registry.get_parameterized(&req.device_id) {
            let params = parameterized.parameters();
            if let Some(param) = params.get(&req.parameter_name) {
                let value = param.get_json().map_err(|e| {
                    map_hardware_error_to_status(&format!("Failed to get parameter: {}", e))
                })?;
                let timestamp_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);

                return Ok(Response::new(ParameterValue {
                    device_id: req.device_id,
                    name: req.parameter_name,
                    value: value.to_string(),
                    units: String::new(), // Could extract from metadata
                    timestamp_ns,
                }));
            }
        }

        // Neither Settable nor Parameterized - device not found
        Err(Status::not_found(format!(
            "Device '{}' does not support parameter '{}'",
            req.device_id, req.parameter_name
        )))
    }

    #[instrument(skip(self, request), fields(method = "set_parameter"))]
    async fn set_parameter(
        &self,
        request: Request<SetParameterRequest>,
    ) -> Result<Response<SetParameterResponse>, Status> {
        let req = request.into_inner();

        // Try legacy Settable trait first (backwards compatibility)
        if let Some(settable) = self.registry.get_settable(&req.device_id) {
            // Get old value before setting (for change notification)
            let old_value = settable
                .get_value(&req.parameter_name)
                .await
                .map(|v| v.to_string())
                .unwrap_or_default();

            // Parse the value string to JSON
            let json_value: serde_json::Value = serde_json::from_str(&req.value)
                .or_else(|_| {
                    // Try as raw string if JSON parsing fails
                    Ok::<_, serde_json::Error>(serde_json::Value::String(req.value.clone()))
                })
                .map_err(|e| Status::invalid_argument(format!("Invalid value format: {}", e)))?;

            // Set the parameter
            settable
                .set_value(&req.parameter_name, json_value)
                .await
                .map_err(|e| Status::invalid_argument(format!("Failed to set parameter: {}", e)))?;

            // Read back the actual value
            let actual_value = settable
                .get_value(&req.parameter_name)
                .await
                .map(|v| v.to_string())
                .unwrap_or_else(|_| req.value.clone());

            // Broadcast parameter change notification (ignore send errors - no subscribers is ok)
            let _ = self.param_change_tx.send(ParameterChange {
                device_id: req.device_id.clone(),
                name: req.parameter_name.clone(),
                old_value,
                new_value: actual_value.clone(),
                units: String::new(), // Would need parameter metadata for units
                timestamp_ns: now_ns(),
                source: "user".to_string(),
            });

            return Ok(Response::new(SetParameterResponse {
                success: true,
                error_message: String::new(),
                actual_value,
            }));
        }

        // New path - use Parameterized trait
        if let Some(parameterized) = self.registry.get_parameterized(&req.device_id) {
            let params = parameterized.parameters();

            if let Some(param) = params.get(&req.parameter_name) {
                let old_value = param.get_json().map(|v| v.to_string()).unwrap_or_default();

                // Parse the value string to JSON
                let json_value: serde_json::Value = serde_json::from_str(&req.value)
                    .or_else(|_| {
                        // Try as raw string if JSON parsing fails
                        Ok::<_, serde_json::Error>(serde_json::Value::String(req.value.clone()))
                    })
                    .map_err(|e| {
                        Status::invalid_argument(format!("Invalid value format: {}", e))
                    })?;

                // Set the parameter (synchronous call, no await needed)
                param.set_json(json_value).map_err(|e| {
                    Status::invalid_argument(format!("Failed to set parameter: {}", e))
                })?;

                let actual_value = param
                    .get_json()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| req.value.clone());

                // Broadcast parameter change notification
                let _ = self.param_change_tx.send(ParameterChange {
                    device_id: req.device_id.clone(),
                    name: req.parameter_name.clone(),
                    old_value,
                    new_value: actual_value.clone(),
                    units: String::new(), // Could get from metadata
                    timestamp_ns: now_ns(),
                    source: "user".to_string(),
                });

                return Ok(Response::new(SetParameterResponse {
                    success: true,
                    error_message: String::new(),
                    actual_value,
                }));
            }
        }

        // Neither Settable nor Parameterized - device not found
        Err(Status::not_found(format!(
            "Device '{}' does not support settable parameters",
            req.device_id
        )))
    }

    type StreamParameterChangesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ParameterChange, Status>>;

    async fn stream_parameter_changes(
        &self,
        request: Request<StreamParameterChangesRequest>,
    ) -> Result<Response<Self::StreamParameterChangesStream>, Status> {
        let req = request.into_inner();

        // Extract filter criteria
        let device_filter = req.device_id.clone();
        let param_filter: std::collections::HashSet<String> =
            req.parameter_names.into_iter().collect();

        // Subscribe to parameter change broadcast
        let mut rx = self.param_change_tx.subscribe();

        // Create mpsc channel for the gRPC stream
        let (tx, stream_rx) = tokio::sync::mpsc::channel(32);

        // Spawn task to forward filtered changes to the stream
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(change) => {
                        // Apply device filter if specified
                        if let Some(ref filter_device) = device_filter
                            && &change.device_id != filter_device
                        {
                            continue;
                        }

                        // Apply parameter name filter if specified
                        if !param_filter.is_empty() && !param_filter.contains(&change.name) {
                            continue;
                        }

                        // Send to stream (exit if receiver dropped)
                        if tx.send(Ok(change)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Parameter change stream lagged, dropped {} messages", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(stream_rx)))
    }

    // =========================================================================
    // Observable Streaming (bd-qqjq, bd-ijre)
    //
    // Dedicated high-throughput stream for numeric observables (sensor readings).
    // Separated from StreamParameterChanges to avoid:
    // 1. Double traffic for rapidly changing values
    // 2. Inefficient string serialization
    // =========================================================================

    type StreamObservablesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ObservableValue, Status>>;

    async fn stream_observables(
        &self,
        request: Request<StreamObservablesRequest>,
    ) -> Result<Response<Self::StreamObservablesStream>, Status> {
        let req = request.into_inner();
        let device_ids = req.device_ids;
        let observable_names = req.observable_names;
        let sample_rate_hz = req.sample_rate_hz.max(1); // Minimum 1 Hz

        // Deadband: minimum change threshold for sending updates (bd-3j0o)
        // Default to 0.001 if not specified or zero, but ensure at least f64::EPSILON
        const DEFAULT_DEADBAND: f64 = 0.001;
        let deadband = if req.deadband <= 0.0 {
            DEFAULT_DEADBAND
        } else {
            req.deadband.max(f64::EPSILON)
        };

        // Calculate sample interval
        let sample_interval = std::time::Duration::from_secs_f64(1.0 / sample_rate_hz as f64);

        // Create output channel
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ObservableValue, Status>>(128);

        // Get registry reference
        let registry = self.registry.clone();

        // Spawn streaming task
        tokio::spawn(async move {
            // Collect observables to monitor
            // Observable uses watch channel (single producer, multiple consumers with latest value)
            let mut subscriptions: Vec<(
                String,                            // device_id
                String,                            // observable_name
                String,                            // units
                tokio::sync::watch::Receiver<f64>, // subscription
                std::time::Instant,                // last_sent
                f64,                               // last_value (for change detection)
            )> = Vec::new();

            for device_id in &device_ids {
                if let Some(parameterized) = registry.get_parameterized(device_id) {
                    let param_set = parameterized.parameters();
                    for obs_name in &observable_names {
                        // Try to get Observable<f64> for this name
                        if let Some(observable) = param_set.get_typed::<Observable<f64>>(obs_name) {
                            let rx = observable.subscribe();
                            let initial_value = *rx.borrow();
                            let units = observable.metadata().units.clone().unwrap_or_default();
                            subscriptions.push((
                                device_id.clone(),
                                obs_name.clone(),
                                units,
                                rx,
                                std::time::Instant::now(),
                                initial_value,
                            ));
                        }
                    }
                }
            }

            if subscriptions.is_empty() {
                tracing::debug!(
                    "StreamObservables: No matching observables found for {:?}/{:?}",
                    device_ids,
                    observable_names
                );
                return;
            }

            tracing::debug!(
                "StreamObservables: Monitoring {} observables at {} Hz",
                subscriptions.len(),
                sample_rate_hz
            );

            // Stream loop - check each subscription for updates
            let mut interval = tokio::time::interval(sample_interval / 2); // Check at 2x rate

            loop {
                interval.tick().await;

                // Check if client disconnected
                if tx.is_closed() {
                    tracing::debug!("StreamObservables: Client disconnected");
                    break;
                }

                // Check each subscription for new values
                for (device_id, obs_name, units, rx, last_sent, last_value) in &mut subscriptions {
                    // Get current value from watch receiver
                    let current_value = *rx.borrow();

                    // Only send if value changed beyond deadband and rate limit elapsed
                    if (current_value - *last_value).abs() > deadband
                        && last_sent.elapsed() >= sample_interval
                    {
                        let msg = ObservableValue {
                            device_id: device_id.clone(),
                            observable_name: obs_name.clone(),
                            value: current_value,
                            units: units.clone(),
                            timestamp_ns: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0),
                        };

                        if tx.send(Ok(msg)).await.is_err() {
                            tracing::debug!("StreamObservables: Failed to send, client gone");
                            return;
                        }

                        *last_sent = std::time::Instant::now();
                        *last_value = current_value;
                    }
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}

// Helper: fetch current device state (shared by SubscribeDeviceState)
async fn fetch_device_state(
    registry: &Arc<DeviceRegistry>,
    device_id: &str,
) -> Result<DeviceStateResponse, Status> {
    // No global lock needed with DashMap
    let (movable, readable, triggerable, frame_producer, exposure_control, exists) = (
        registry.get_movable(device_id),
        registry.get_readable(device_id),
        registry.get_triggerable(device_id),
        registry.get_frame_producer(device_id),
        registry.get_exposure_control(device_id),
        registry.contains(device_id),
    );

    if !exists {
        return Err(Status::not_found(format!(
            "Device not found: {}",
            device_id
        )));
    }

    let mut response = DeviceStateResponse {
        device_id: device_id.to_string(),
        online: true,
        position: None,
        last_reading: None,
        armed: None,
        streaming: None,
        exposure_ms: None,
    };

    if let Some(movable) = movable
        && let Ok(pos) = movable.position().await
    {
        response.position = Some(pos);
    }
    if let Some(readable) = readable
        && let Ok(val) = readable.read().await
    {
        response.last_reading = Some(val);
    }
    if let Some(triggerable) = triggerable {
        response.armed = triggerable.is_armed().await.ok();
    }
    if let Some(frame_producer) = frame_producer {
        response.streaming = frame_producer.is_streaming().await.ok();
    }
    if let Some(exposure_ctrl) = exposure_control
        && let Ok(seconds) = exposure_ctrl.get_exposure().await
    {
        response.exposure_ms = Some(seconds * 1000.0);
    }

    Ok(response)
}

// Helper: convert state to sparse field map
fn device_state_to_fields_json(state: &DeviceStateResponse) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("online".into(), state.online.to_string());
    if let Some(p) = state.position {
        map.insert("position".into(), p.to_string());
    }
    if let Some(r) = state.last_reading {
        map.insert("reading".into(), r.to_string());
    }
    if let Some(a) = state.armed {
        map.insert("armed".into(), a.to_string());
    }
    if let Some(s) = state.streaming {
        map.insert("streaming".into(), s.to_string());
    }
    if let Some(e) = state.exposure_ms {
        map.insert("exposure_ms".into(), e.to_string());
    }
    map
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn monitor_parameter<T: std::fmt::Display + Clone + Send + Sync + 'static>(
    mut rx: tokio::sync::watch::Receiver<T>,
    tx: tokio::sync::broadcast::Sender<ParameterChange>,
    device_id: String,
    name: String,
) {
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let value = rx.borrow().clone();
            let change = ParameterChange {
                device_id: device_id.clone(),
                name: name.clone(),
                old_value: String::new(),
                new_value: value.to_string(),
                units: String::new(),
                timestamp_ns: now_ns(),
                source: "hardware".to_string(),
            };
            let _ = tx.send(change);
        }
    });
}

/// Map hardware errors to canonical gRPC Status codes
///
/// This function provides consistent error semantics across all hardware RPCs.
/// Maps error messages to appropriate Status codes:
/// - Device not found → NOT_FOUND
/// - Device busy/armed/streaming state → FAILED_PRECONDITION
/// - Communication error → UNAVAILABLE
/// - Invalid parameter → INVALID_ARGUMENT
/// - Operation not supported → UNIMPLEMENTED
fn map_hardware_error_to_status(error_msg: &str) -> Status {
    let err_lower = error_msg.to_lowercase();

    if err_lower.contains("not found") || err_lower.contains("no such device") {
        Status::not_found(error_msg.to_string())
    } else if err_lower.contains("busy")
        || err_lower.contains("in use")
        || err_lower.contains("already")
        || err_lower.contains("not armed")
        || err_lower.contains("not streaming")
        || err_lower.contains("streaming")
        || err_lower.contains("precondition")
    {
        Status::failed_precondition(error_msg.to_string())
    } else if err_lower.contains("timeout")
        || err_lower.contains("communication")
        || err_lower.contains("connection")
    {
        Status::unavailable(error_msg.to_string())
    } else if err_lower.contains("invalid")
        || err_lower.contains("out of range")
        || err_lower.contains("bounds")
    {
        Status::invalid_argument(error_msg.to_string())
    } else if err_lower.contains("not supported") || err_lower.contains("unsupported") {
        Status::unimplemented(error_msg.to_string())
    } else {
        // Default to INTERNAL for unknown errors
        Status::internal(error_msg.to_string())
    }
}

/// Convert internal DeviceInfo to proto DeviceInfo
fn device_info_to_proto(info: &daq_hardware::registry::DeviceInfo) -> DeviceInfo {
    // Use explicit category from metadata if set, otherwise infer from driver/capabilities
    let category = get_device_category(
        info.metadata.category,
        &info.driver_type,
        &info.capabilities,
    );

    DeviceInfo {
        id: info.id.clone(),
        name: info.name.clone(),
        driver_type: info.driver_type.clone(),
        category: category as i32,
        is_movable: info.capabilities.contains(&Capability::Movable),
        is_readable: info.capabilities.contains(&Capability::Readable),
        is_triggerable: info.capabilities.contains(&Capability::Triggerable),
        is_frame_producer: info.capabilities.contains(&Capability::FrameProducer),
        is_exposure_controllable: info.capabilities.contains(&Capability::ExposureControl),
        // Laser control capabilities (bd-pwjo)
        is_shutter_controllable: info.capabilities.contains(&Capability::ShutterControl),
        is_wavelength_tunable: info.capabilities.contains(&Capability::WavelengthTunable),
        is_emission_controllable: info.capabilities.contains(&Capability::EmissionControl),
        metadata: Some(ProtoDeviceMetadata {
            position_units: info.metadata.position_units.clone(),
            min_position: info.metadata.min_position,
            max_position: info.metadata.max_position,
            reading_units: info.metadata.measurement_units.clone(),
            frame_width: info.metadata.frame_width,
            frame_height: info.metadata.frame_height,
            bits_per_pixel: info.metadata.bits_per_pixel,
            min_exposure_ms: info.metadata.min_exposure_ms,
            max_exposure_ms: info.metadata.max_exposure_ms,
            // Wavelength limits for tunable lasers (bd-pwjo)
            min_wavelength_nm: info.metadata.min_wavelength_nm,
            max_wavelength_nm: info.metadata.max_wavelength_nm,
        }),
    }
}

/// Get device category, preferring explicit metadata over inference (bd-le6k)
///
/// Priority:
/// 1. Explicit category from DeviceMetadata (set by driver)
/// 2. String-based inference from driver_type (fallback)
/// 3. Capability-based inference (last resort)
fn get_device_category(
    explicit_category: Option<daq_core::capabilities::DeviceCategory>,
    driver_type: &str,
    capabilities: &[Capability],
) -> daq_proto::DeviceCategory {
    use daq_core::capabilities::DeviceCategory as CoreCategory;
    use daq_proto::DeviceCategory as ProtoCategory;

    // 1. Use explicit category from metadata if set by driver
    if let Some(category) = explicit_category {
        return match category {
            CoreCategory::Camera => ProtoCategory::Camera,
            CoreCategory::Stage => ProtoCategory::Stage,
            CoreCategory::Detector => ProtoCategory::Detector,
            CoreCategory::Laser => ProtoCategory::Laser,
            CoreCategory::PowerMeter => ProtoCategory::PowerMeter,
            CoreCategory::Other => ProtoCategory::Other,
        };
    }

    // 2. Fall back to string-based inference from driver type
    let driver_lower = driver_type.to_lowercase();

    if driver_lower.contains("pvcam") || driver_lower.contains("camera") {
        return ProtoCategory::Camera;
    }

    if driver_lower.contains("maitai") || driver_lower.contains("laser") {
        return ProtoCategory::Laser;
    }

    if driver_lower.contains("1830")
        || driver_lower.contains("power_meter")
        || driver_lower.contains("powermeter")
    {
        return ProtoCategory::PowerMeter;
    }

    if driver_lower.contains("esp300")
        || driver_lower.contains("ell14")
        || driver_lower.contains("stage")
    {
        return ProtoCategory::Stage;
    }

    // 3. Fall back to capability-based inference
    if capabilities.contains(&Capability::FrameProducer) {
        return ProtoCategory::Camera;
    }

    if capabilities.contains(&Capability::WavelengthTunable)
        || capabilities.contains(&Capability::EmissionControl)
    {
        return ProtoCategory::Laser;
    }

    if capabilities.contains(&Capability::Movable) {
        return ProtoCategory::Stage;
    }

    if capabilities.contains(&Capability::Readable) && !capabilities.contains(&Capability::Movable)
    {
        return ProtoCategory::Detector;
    }

    // Default to Other for unknown devices
    ProtoCategory::Other
}

#[cfg(test)]
mod tests {
    use super::*;
    use daq_hardware::registry::create_mock_registry;

    #[tokio::test]
    async fn test_list_devices() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ListDevicesRequest {
            capability_filter: None,
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 3);

        // Verify expected devices are present
        let device_ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert!(device_ids.contains(&"mock_stage"));
        assert!(device_ids.contains(&"mock_power_meter"));
        assert!(device_ids.contains(&"mock_camera"));
    }

    #[tokio::test]
    async fn test_list_devices_with_filter() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Filter for movable devices
        let request = Request::new(ListDevicesRequest {
            capability_filter: Some("movable".to_string()),
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 1);
        assert!(devices[0].is_movable);
    }

    #[tokio::test]
    async fn test_move_absolute() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(MoveRequest {
            device_id: "mock_stage".to_string(),
            value: 10.0,
            wait_for_completion: None,
            timeout_ms: None,
        });
        let response = service.move_absolute(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!((resp.final_position - 10.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_read_value() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ReadValueRequest {
            device_id: "mock_power_meter".to_string(),
        });
        let response = service.read_value(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(resp.value > 0.0);
    }

    #[tokio::test]
    async fn test_device_not_found() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(MoveRequest {
            device_id: "nonexistent".to_string(),
            value: 10.0,
            wait_for_completion: None,
            timeout_ms: None,
        });
        let result = service.move_absolute(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_wrong_capability() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Try to move the power meter (not movable)
        let request = Request::new(MoveRequest {
            device_id: "mock_power_meter".to_string(),
            value: 10.0,
            wait_for_completion: None,
            timeout_ms: None,
        });
        let result = service.move_absolute(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_move_with_wait_for_completion() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(MoveRequest {
            device_id: "mock_stage".to_string(),
            value: 25.0,
            wait_for_completion: Some(true),
            timeout_ms: Some(5000),
        });
        let response = service.move_absolute(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!((resp.final_position - 25.0).abs() < 0.001);
        assert_eq!(resp.settled, Some(true));
    }

    // =========================================================================
    // Stage/Unstage Tests (bd-h917)
    // =========================================================================

    #[tokio::test]
    async fn test_stage_device_success() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(StageDeviceRequest {
            device_id: "mock_stage".to_string(),
        });
        let response = service.stage_device(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(resp.staged);
        assert!(resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_stage_device_not_found() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(StageDeviceRequest {
            device_id: "nonexistent".to_string(),
        });
        let result = service.stage_device(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_unstage_device_success() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(UnstageDeviceRequest {
            device_id: "mock_power_meter".to_string(),
        });
        let response = service.unstage_device(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_unstage_device_not_found() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(UnstageDeviceRequest {
            device_id: "nonexistent".to_string(),
        });
        let result = service.unstage_device(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    // =========================================================================
    // Streaming Tests (bd-9pss)
    // =========================================================================

    #[tokio::test]
    async fn test_subscribe_device_state_success() {
        use tokio_stream::StreamExt;

        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(DeviceStateSubscribeRequest {
            device_ids: vec!["mock_stage".to_string()],
            max_rate_hz: 10,
            last_seen_version: 0,
            include_snapshot: true,
        });
        let response = service.subscribe_device_state(request).await.unwrap();
        let mut stream = response.into_inner();

        // Receive at least one state update
        let update = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timeout waiting for state update");

        assert!(update.is_some());
        let state = update.unwrap().expect("stream item should be Ok");
        assert_eq!(state.device_id, "mock_stage");
    }

    #[tokio::test]
    async fn test_subscribe_device_state_not_found() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(DeviceStateSubscribeRequest {
            device_ids: vec!["nonexistent".to_string()],
            max_rate_hz: 10,
            last_seen_version: 0,
            include_snapshot: false,
        });
        let result = service.subscribe_device_state(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_stream_parameter_changes() {
        use tokio_stream::StreamExt;

        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Get the parameter change sender to simulate changes
        let param_sender = service.param_change_sender();

        // Start streaming (no filters)
        let request = Request::new(StreamParameterChangesRequest {
            device_id: None,
            parameter_names: vec![],
        });
        let response = service.stream_parameter_changes(request).await.unwrap();
        let mut stream = response.into_inner();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Send a parameter change
        let _ = param_sender.send(ParameterChange {
            device_id: "mock_stage".to_string(),
            name: "position".to_string(),
            old_value: String::new(), // Not available in listener callback
            new_value: "10.5".to_string(),
            units: String::new(), // Could get from metadata if needed
            timestamp_ns: now_ns(),
            source: "user".to_string(),
        });

        // Receive the change
        let change = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timeout waiting for parameter change");

        assert!(change.is_some());
        let change_data = change.unwrap().expect("stream item should be Ok");
        assert_eq!(change_data.device_id, "mock_stage");
        assert_eq!(change_data.name, "position");
        assert_eq!(change_data.new_value, "10.5");
    }

    #[tokio::test]
    async fn test_stream_parameter_changes_with_filter() {
        use tokio_stream::StreamExt;

        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));
        let param_sender = service.param_change_sender();

        // Start streaming with device filter
        let request = Request::new(StreamParameterChangesRequest {
            device_id: Some("mock_camera".to_string()),
            parameter_names: vec![],
        });
        let response = service.stream_parameter_changes(request).await.unwrap();
        let mut stream = response.into_inner();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Send a change for mock_stage (should be filtered out)
        let _ = param_sender.send(ParameterChange {
            device_id: "mock_stage".to_string(),
            name: "position".to_string(),
            old_value: String::new(), // Not available in listener callback
            new_value: "5.0".to_string(),
            units: String::new(), // Could get from metadata if needed
            timestamp_ns: now_ns(),
            source: "user".to_string(),
        });

        // Send a change for mock_camera (should pass filter)
        let _ = param_sender.send(ParameterChange {
            device_id: "mock_camera".to_string(),
            name: "exposure".to_string(),
            old_value: String::new(), // Not available in listener callback
            new_value: "0.5".to_string(),
            units: String::new(), // Could get from metadata if needed
            timestamp_ns: now_ns(),
            source: "user".to_string(),
        });

        // Should receive only the camera change
        let change = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timeout waiting for parameter change");

        assert!(change.is_some());
        let change_data = change.unwrap().expect("stream item should be Ok");
        assert_eq!(change_data.device_id, "mock_camera");
        assert_eq!(change_data.name, "exposure");
    }

    #[tokio::test]
    async fn test_list_parameters_v5() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // List parameters for mock_stage
        let request = Request::new(ListParametersRequest {
            device_id: "mock_stage".to_string(),
        });
        let response = service.list_parameters(request).await.unwrap();
        let parameters = response.into_inner().parameters;

        // Verify "position" parameter is present
        let position_param = parameters.iter().find(|p| p.name == "position");
        assert!(position_param.is_some(), "position parameter not found");

        let p = position_param.unwrap();
        assert_eq!(p.device_id, "mock_stage");
        assert_eq!(p.dtype, "float"); // inferred from f64
        assert!(p.writable);
        assert!(p.readable);
        // units might differ based on mock implementation details
    }
}
