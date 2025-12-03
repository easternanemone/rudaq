//! HardwareService implementation for direct device control (bd-4x6q)
//!
//! This module provides gRPC endpoints for direct hardware manipulation,
//! bypassing the scripting layer. It connects to the DeviceRegistry for
//! capability-based access to hardware devices.

use crate::grpc::proto::{
    hardware_service_server::HardwareService,
    ArmRequest,
    ArmResponse,
    DeviceCommandRequest,
    DeviceCommandResponse,
    DeviceInfo,
    DeviceMetadata as ProtoDeviceMetadata,
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
    ListParametersRequest,
    ListParametersResponse,
    MoveRequest,
    MoveResponse,
    ParameterChange,
    ParameterDescriptor,
    ParameterValue,
    PositionUpdate,
    ReadValueRequest,
    ReadValueResponse,
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
    StreamFramesRequest,
    StreamParameterChangesRequest,
    StreamPositionRequest,
    StreamValuesRequest,
    TriggerRequest,
    TriggerResponse,
    UnstageDeviceRequest,
    UnstageDeviceResponse,
    ValueUpdate,
    WaitSettledRequest,
    WaitSettledResponse,
};
use crate::hardware::registry::{Capability, DeviceRegistry};
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

/// Hardware gRPC service implementation
///
/// Provides direct access to hardware devices through the DeviceRegistry.
/// All hardware operations are delegated to the appropriate capability traits.
pub struct HardwareServiceImpl {
    registry: Arc<RwLock<DeviceRegistry>>,
    /// Broadcast sender for parameter changes (enables real-time GUI synchronization)
    param_change_tx: tokio::sync::broadcast::Sender<ParameterChange>,
}

impl HardwareServiceImpl {
    /// Create a new HardwareService with the given device registry
    pub fn new(registry: Arc<RwLock<DeviceRegistry>>) -> Self {
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

            let reg = registry_clone.read().await;

            // Iterate all devices and spawn monitors for parameters
            for device_info in reg.list_devices() {
                let device_id = device_info.id.clone();

                if let Some(param_set) = reg.get_parameters(&device_id) {
                    // Found a Parameterized device - monitor all its parameters
                    for param_name in param_set.names() {
                        let tx = tx_clone.clone();
                        let dev_id = device_id.clone();
                        let p_name = param_name.to_string();

                        // Try to downcast to Observable<f64> (most common parameter type)
                        if let Some(obs_f64) = param_set.get_typed::<f64>(param_name) {
                            let mut rx = obs_f64.subscribe();
                            tokio::spawn(async move {
                                while rx.changed().await.is_ok() {
                                    let value = *rx.borrow();
                                    let change = ParameterChange {
                                        device_id: dev_id.clone(),
                                        name: p_name.clone(),
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
                        // TODO(bd-zafg): Add downcasts for other common types (bool, String, etc.)
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
        registry: Arc<RwLock<DeviceRegistry>>,
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

    async fn list_devices(
        &self,
        request: Request<ListDevicesRequest>,
    ) -> Result<Response<ListDevicesResponse>, Status> {
        let req = request.into_inner();
        let registry = self.registry.read().await;

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

            registry
                .devices_with_capability(cap)
                .iter()
                .filter_map(|id| registry.get_device_info(id))
                .map(|info| device_info_to_proto(&info))
                .collect()
        } else {
            // Return all devices
            registry
                .list_devices()
                .iter()
                .map(|info| device_info_to_proto(info))
                .collect()
        };

        Ok(Response::new(ListDevicesResponse { devices }))
    }

    async fn get_device_state(
        &self,
        request: Request<DeviceStateRequest>,
    ) -> Result<Response<DeviceStateResponse>, Status> {
        let req = request.into_inner();

        // Acquire lock, extract Arc references, then release lock before awaiting
        // This prevents deadlock when hardware operations take time
        let (movable, readable, triggerable, frame_producer, exposure_control, exists) = {
            let registry = self.registry.read().await;
            (
                registry.get_movable(&req.device_id),
                registry.get_readable(&req.device_id),
                registry.get_triggerable(&req.device_id),
                registry.get_frame_producer(&req.device_id),
                registry.get_exposure_control(&req.device_id),
                registry.contains(&req.device_id),
            )
        }; // Lock released here

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
                Ok(pos) => response.position = Some(pos),
                Err(_) => response.online = false,
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

        if let Some(exposure_ctrl) = exposure_control {
            if let Ok(seconds) = exposure_ctrl.get_exposure().await {
                response.exposure_ms = Some(seconds * 1000.0);
            }
        }

        Ok(Response::new(response))
    }

    async fn subscribe_device_state(
        &self,
        request: Request<DeviceStateSubscribeRequest>,
    ) -> Result<Response<Self::SubscribeDeviceStateStream>, Status> {
        let req = request.into_inner();

        // Determine device list and validate device IDs exist
        let device_ids: Vec<String> = {
            let registry = self.registry.read().await;
            if req.device_ids.is_empty() {
                registry
                    .list_devices()
                    .iter()
                    .map(|d| d.id.clone())
                    .collect()
            } else {
                // Validate all requested device IDs exist
                for device_id in &req.device_ids {
                    if !registry.contains(device_id) {
                        return Err(Status::not_found(format!(
                            "Device '{}' not found",
                            device_id
                        )));
                    }
                }
                req.device_ids.clone()
            }
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

    async fn move_absolute(
        &self,
        request: Request<MoveRequest>,
    ) -> Result<Response<MoveResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let movable = {
            let registry = self.registry.read().await;
            registry.get_movable(&req.device_id)
        };

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        // Send move command
        match movable.move_abs(req.value).await {
            Ok(_) => {
                // Check if we should wait for completion
                let (final_position, settled) = if req.wait_for_completion.unwrap_or(false) {
                    // Wait for motion to complete with optional timeout
                    let settle_result = if let Some(timeout_ms) = req.timeout_ms {
                        tokio::time::timeout(
                            Duration::from_millis(timeout_ms as u64),
                            movable.wait_settled(),
                        )
                        .await
                    } else {
                        Ok(movable.wait_settled().await)
                    };

                    match settle_result {
                        Ok(Ok(_)) => {
                            let pos = movable.position().await.unwrap_or(req.value);
                            (pos, Some(true))
                        }
                        Ok(Err(e)) => {
                            // wait_settled failed
                            return Err(map_hardware_error_to_status(&e.to_string()));
                        }
                        Err(_) => {
                            // Timeout
                            let pos = movable.position().await.unwrap_or(req.value);
                            return Err(Status::deadline_exceeded(format!(
                                "Motion did not complete within {} ms, current position: {}",
                                req.timeout_ms.unwrap_or(0),
                                pos
                            )));
                        }
                    }
                } else {
                    // Return immediately without waiting
                    let pos = movable.position().await.unwrap_or(req.value);
                    (pos, None)
                };

                Ok(Response::new(MoveResponse {
                    success: true,
                    error_message: String::new(),
                    final_position,
                    settled,
                }))
            }
            Err(e) => {
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }

    async fn move_relative(
        &self,
        request: Request<MoveRequest>,
    ) -> Result<Response<MoveResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let movable = {
            let registry = self.registry.read().await;
            registry.get_movable(&req.device_id)
        };

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        // Send move command
        match movable.move_rel(req.value).await {
            Ok(_) => {
                // Check if we should wait for completion
                let (final_position, settled) = if req.wait_for_completion.unwrap_or(false) {
                    // Wait for motion to complete with optional timeout
                    let settle_result = if let Some(timeout_ms) = req.timeout_ms {
                        tokio::time::timeout(
                            Duration::from_millis(timeout_ms as u64),
                            movable.wait_settled(),
                        )
                        .await
                    } else {
                        Ok(movable.wait_settled().await)
                    };

                    match settle_result {
                        Ok(Ok(_)) => {
                            let pos = movable.position().await.unwrap_or(0.0);
                            (pos, Some(true))
                        }
                        Ok(Err(e)) => {
                            return Err(map_hardware_error_to_status(&e.to_string()));
                        }
                        Err(_) => {
                            let pos = movable.position().await.unwrap_or(0.0);
                            return Err(Status::deadline_exceeded(format!(
                                "Motion did not complete within {} ms, current position: {}",
                                req.timeout_ms.unwrap_or(0),
                                pos
                            )));
                        }
                    }
                } else {
                    // Return immediately without waiting
                    let pos = movable.position().await.unwrap_or(0.0);
                    (pos, None)
                };

                Ok(Response::new(MoveResponse {
                    success: true,
                    error_message: String::new(),
                    final_position,
                    settled,
                }))
            }
            Err(e) => {
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }

    async fn stop_motion(
        &self,
        request: Request<StopMotionRequest>,
    ) -> Result<Response<StopMotionResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let movable = {
            let registry = self.registry.read().await;
            registry.get_movable(&req.device_id)
        };

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        // Call the actual stop method on the Movable trait
        match movable.stop().await {
            Ok(_) => {
                // Get the stopped position
                let position = movable.position().await.unwrap_or(0.0);
                Ok(Response::new(StopMotionResponse {
                    success: true,
                    stopped_position: position,
                }))
            }
            Err(e) => {
                // Stop not supported or hardware error
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }

    async fn wait_settled(
        &self,
        request: Request<WaitSettledRequest>,
    ) -> Result<Response<WaitSettledResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let movable = {
            let registry = self.registry.read().await;
            registry.get_movable(&req.device_id)
        };

        let movable = movable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not movable",
                req.device_id
            ))
        })?;

        // Apply timeout if specified
        let settle_future = movable.wait_settled();
        let result = if let Some(timeout_ms) = req.timeout_ms {
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms as u64),
                settle_future,
            )
            .await
        } else {
            Ok(settle_future.await)
        };

        match result {
            Ok(Ok(_)) => {
                let position = movable.position().await.unwrap_or(0.0);
                Ok(Response::new(WaitSettledResponse {
                    success: true,
                    settled: true,
                    position,
                }))
            }
            Ok(Err(e)) => {
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
            Err(_) => {
                // Timeout occurred
                Err(Status::deadline_exceeded(format!(
                    "Wait settled operation timed out for device '{}'",
                    req.device_id
                )))
            }
        }
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
        {
            let reg = registry.read().await;
            if reg.get_movable(&device_id).is_none() {
                return Err(Status::not_found(format!(
                    "Device '{}' not found or not movable",
                    device_id
                )));
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs_f64(1.0 / rate_hz as f64);
            let mut ticker = tokio::time::interval(interval);
            let mut last_position = f64::NAN;

            loop {
                ticker.tick().await;

                // Extract Arc and release lock before awaiting hardware
                let movable = {
                    let reg = registry.read().await;
                    reg.get_movable(&device_id)
                };

                if let Some(movable) = movable {
                    let position = movable.position().await.unwrap_or(f64::NAN);
                    let is_moving = (position - last_position).abs() > 0.0001;
                    last_position = position;

                    let update = PositionUpdate {
                        device_id: device_id.clone(),
                        position,
                        timestamp_ns: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
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

    async fn read_value(
        &self,
        request: Request<ReadValueRequest>,
    ) -> Result<Response<ReadValueResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and metadata, then release lock before awaiting hardware
        let (readable, units) = {
            let registry = self.registry.read().await;
            let readable = registry.get_readable(&req.device_id);
            let units = registry
                .get_device_info(&req.device_id)
                .and_then(|info| info.metadata.measurement_units.clone())
                .unwrap_or_default();
            (readable, units)
        };

        let readable = readable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not readable",
                req.device_id
            ))
        })?;

        match readable.read().await {
            Ok(value) => Ok(Response::new(ReadValueResponse {
                success: true,
                error_message: String::new(),
                value,
                units,
                timestamp_ns: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64,
            })),
            Err(e) => {
                let err_msg = e.to_string();
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
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
        {
            let reg = registry.read().await;
            if reg.get_readable(&device_id).is_none() {
                return Err(Status::not_found(format!(
                    "Device '{}' not found or not readable",
                    device_id
                )));
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs_f64(1.0 / rate_hz as f64);
            let mut ticker = tokio::time::interval(interval);

            loop {
                ticker.tick().await;

                // Extract Arc and metadata, release lock before awaiting hardware
                let (readable, units) = {
                    let reg = registry.read().await;
                    let readable = reg.get_readable(&device_id);
                    let units = reg
                        .get_device_info(&device_id)
                        .and_then(|info| info.metadata.measurement_units.clone())
                        .unwrap_or_default();
                    (readable, units)
                };

                if let Some(readable) = readable {
                    if let Ok(value) = readable.read().await {
                        let update = ValueUpdate {
                            device_id: device_id.clone(),
                            value,
                            units,
                            timestamp_ns: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
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

    async fn arm(&self, request: Request<ArmRequest>) -> Result<Response<ArmResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let triggerable = {
            let registry = self.registry.read().await;
            registry.get_triggerable(&req.device_id)
        };

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

    async fn trigger(
        &self,
        request: Request<TriggerRequest>,
    ) -> Result<Response<TriggerResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let triggerable = {
            let registry = self.registry.read().await;
            registry.get_triggerable(&req.device_id)
        };

        let triggerable = triggerable.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not triggerable",
                req.device_id
            ))
        })?;

        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
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

    async fn set_exposure(
        &self,
        request: Request<SetExposureRequest>,
    ) -> Result<Response<SetExposureResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let exposure_ctrl = {
            let registry = self.registry.read().await;
            registry.get_exposure_control(&req.device_id)
        };

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

        // Extract Arc and release lock before awaiting hardware
        let exposure_ctrl = {
            let registry = self.registry.read().await;
            registry.get_exposure_control(&req.device_id)
        };

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
            Err(e) => Err(Status::internal(format!("Failed to get exposure: {}", e))),
        }
    }

    // =========================================================================
    // Laser Control (bd-pwjo)
    // =========================================================================

    async fn set_shutter(
        &self,
        request: Request<SetShutterRequest>,
    ) -> Result<Response<SetShutterResponse>, Status> {
        let req = request.into_inner();

        #[cfg(feature = "instrument_spectra_physics")]
        {
            let shutter_ctrl = {
                let registry = self.registry.read().await;
                registry.get_shutter_control(&req.device_id)
            };

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
                Err(e) => Err(Status::internal(format!("Failed to set shutter: {}", e))),
            }
        }

        #[cfg(not(feature = "instrument_spectra_physics"))]
        {
            let _ = req;
            Err(Status::unimplemented(
                "Shutter control requires instrument_spectra_physics feature",
            ))
        }
    }

    async fn get_shutter(
        &self,
        request: Request<GetShutterRequest>,
    ) -> Result<Response<GetShutterResponse>, Status> {
        let req = request.into_inner();

        #[cfg(feature = "instrument_spectra_physics")]
        {
            let shutter_ctrl = {
                let registry = self.registry.read().await;
                registry.get_shutter_control(&req.device_id)
            };

            let shutter_ctrl = shutter_ctrl.ok_or_else(|| {
                Status::not_found(format!(
                    "Device '{}' not found or has no shutter control",
                    req.device_id
                ))
            })?;

            match shutter_ctrl.is_shutter_open().await {
                Ok(is_open) => Ok(Response::new(GetShutterResponse { is_open })),
                Err(e) => Err(Status::internal(format!(
                    "Failed to get shutter state: {}",
                    e
                ))),
            }
        }

        #[cfg(not(feature = "instrument_spectra_physics"))]
        {
            let _ = req;
            Err(Status::unimplemented(
                "Shutter control requires instrument_spectra_physics feature",
            ))
        }
    }

    async fn set_wavelength(
        &self,
        request: Request<SetWavelengthRequest>,
    ) -> Result<Response<SetWavelengthResponse>, Status> {
        let req = request.into_inner();

        #[cfg(feature = "instrument_spectra_physics")]
        {
            let wavelength_ctrl = {
                let registry = self.registry.read().await;
                registry.get_wavelength_tunable(&req.device_id)
            };

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
                Err(e) => Err(Status::internal(format!("Failed to set wavelength: {}", e))),
            }
        }

        #[cfg(not(feature = "instrument_spectra_physics"))]
        {
            let _ = req;
            Err(Status::unimplemented(
                "Wavelength control requires instrument_spectra_physics feature",
            ))
        }
    }

    async fn get_wavelength(
        &self,
        request: Request<GetWavelengthRequest>,
    ) -> Result<Response<GetWavelengthResponse>, Status> {
        let req = request.into_inner();

        #[cfg(feature = "instrument_spectra_physics")]
        {
            let wavelength_ctrl = {
                let registry = self.registry.read().await;
                registry.get_wavelength_tunable(&req.device_id)
            };

            let wavelength_ctrl = wavelength_ctrl.ok_or_else(|| {
                Status::not_found(format!(
                    "Device '{}' not found or has no wavelength control",
                    req.device_id
                ))
            })?;

            match wavelength_ctrl.get_wavelength().await {
                Ok(nm) => Ok(Response::new(GetWavelengthResponse { wavelength_nm: nm })),
                Err(e) => Err(Status::internal(format!("Failed to get wavelength: {}", e))),
            }
        }

        #[cfg(not(feature = "instrument_spectra_physics"))]
        {
            let _ = req;
            Err(Status::unimplemented(
                "Wavelength control requires instrument_spectra_physics feature",
            ))
        }
    }

    async fn set_emission(
        &self,
        request: Request<SetEmissionRequest>,
    ) -> Result<Response<SetEmissionResponse>, Status> {
        let req = request.into_inner();

        #[cfg(feature = "instrument_spectra_physics")]
        {
            let emission_ctrl = {
                let registry = self.registry.read().await;
                registry.get_emission_control(&req.device_id)
            };

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
                Err(e) => Err(Status::internal(format!("Failed to set emission: {}", e))),
            }
        }

        #[cfg(not(feature = "instrument_spectra_physics"))]
        {
            let _ = req;
            Err(Status::unimplemented(
                "Emission control requires instrument_spectra_physics feature",
            ))
        }
    }

    async fn get_emission(
        &self,
        request: Request<GetEmissionRequest>,
    ) -> Result<Response<GetEmissionResponse>, Status> {
        let req = request.into_inner();

        #[cfg(feature = "instrument_spectra_physics")]
        {
            let emission_ctrl = {
                let registry = self.registry.read().await;
                registry.get_emission_control(&req.device_id)
            };

            let emission_ctrl = emission_ctrl.ok_or_else(|| {
                Status::not_found(format!(
                    "Device '{}' not found or has no emission control",
                    req.device_id
                ))
            })?;

            match emission_ctrl.is_emission_enabled().await {
                Ok(is_enabled) => Ok(Response::new(GetEmissionResponse { is_enabled })),
                Err(e) => Err(Status::internal(format!(
                    "Failed to get emission state: {}",
                    e
                ))),
            }
        }

        #[cfg(not(feature = "instrument_spectra_physics"))]
        {
            let _ = req;
            Err(Status::unimplemented(
                "Emission control requires instrument_spectra_physics feature",
            ))
        }
    }

    // =========================================================================
    // Frame Streaming
    // =========================================================================

    async fn start_stream(
        &self,
        request: Request<StartStreamRequest>,
    ) -> Result<Response<StartStreamResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let frame_producer = {
            let registry = self.registry.read().await;
            registry.get_frame_producer(&req.device_id)
        };

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
                // Check for already streaming
                if err_msg.to_lowercase().contains("already streaming") {
                    Err(Status::failed_precondition(
                        "Device is already streaming; stop current stream first",
                    ))
                } else {
                    let status = map_hardware_error_to_status(&err_msg);
                    Err(status)
                }
            }
        }
    }

    async fn stop_stream(
        &self,
        request: Request<StopStreamRequest>,
    ) -> Result<Response<StopStreamResponse>, Status> {
        let req = request.into_inner();

        // Extract Arc and release lock before awaiting hardware
        let frame_producer = {
            let registry = self.registry.read().await;
            registry.get_frame_producer(&req.device_id)
        };

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
            Err(e) => Err(Status::internal(format!("Failed to stop stream: {}", e))),
        }
    }

    type StreamFramesStream =
        tokio_stream::wrappers::ReceiverStream<Result<crate::grpc::proto::FrameData, Status>>;

    async fn stream_frames(
        &self,
        request: Request<StreamFramesRequest>,
    ) -> Result<Response<Self::StreamFramesStream>, Status> {
        let req = request.into_inner();
        let include_pixel_data = req.include_pixel_data.unwrap_or(true);
        let device_id = req.device_id.clone();

        // Extract FrameProducer Arc, then release lock before taking receiver
        let frame_producer = {
            let registry = self.registry.read().await;
            registry.get_frame_producer(&device_id)
        };

        let frame_producer = frame_producer.ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or not a frame producer",
                device_id
            ))
        })?;

        // Start streaming if not already active
        frame_producer.start_stream().await.map_err(|e| {
            Status::internal(format!("Failed to start stream for '{}': {}", device_id, e))
        })?;

        // Subscribe to frame broadcasts (can be called multiple times for multiple subscribers)
        let mut frame_rx = frame_producer.subscribe_frames().await.ok_or_else(|| {
            Status::failed_precondition(format!(
                "Frame streaming for '{}' is not available or not supported",
                device_id
            ))
        })?;

        // Create gRPC output channel
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<crate::grpc::proto::FrameData, Status>>(32);

        // Spawn background task to convert Frame → FrameData and forward to gRPC stream
        tokio::spawn(async move {
            let mut frame_number: u32 = 0;

            while let Ok(frame) = frame_rx.recv().await {
                // Convert Frame to FrameData proto
                let pixel_data = if include_pixel_data {
                    // Convert Vec<u16> to bytes (little-endian)
                    let mut bytes = Vec::with_capacity(frame.buffer.len() * 2);
                    for pixel in &frame.buffer {
                        bytes.extend_from_slice(&pixel.to_le_bytes());
                    }
                    bytes
                } else {
                    Vec::new()
                };

                let frame_data = crate::grpc::proto::FrameData {
                    device_id: device_id.clone(),
                    frame_number,
                    width: frame.width,
                    height: frame.height,
                    timestamp_ns: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos() as u64,
                    pixel_data,
                    pixel_format: if include_pixel_data {
                        "u16_le".to_string()
                    } else {
                        String::new()
                    },
                    // Arrow Flight ticket for bulk data transfer (not used for gRPC streaming)
                    flight_ticket: None,
                };

                frame_number = frame_number.wrapping_add(1);

                if tx.send(Ok(frame_data)).await.is_err() {
                    // Client disconnected
                    break;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
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
    async fn stage_device(
        &self,
        request: Request<StageDeviceRequest>,
    ) -> Result<Response<StageDeviceResponse>, Status> {
        let req = request.into_inner();
        let registry = self.registry.read().await;

        // Verify device exists
        if !registry.contains(&req.device_id) {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        // If device implements Stageable, call stage()
        if let Some(stageable) = registry.get_stageable(&req.device_id) {
            stageable.stage().await.map_err(|e| {
                Status::internal(format!("Failed to stage device '{}': {}", req.device_id, e))
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
    async fn unstage_device(
        &self,
        request: Request<UnstageDeviceRequest>,
    ) -> Result<Response<UnstageDeviceResponse>, Status> {
        let req = request.into_inner();
        let registry = self.registry.read().await;

        // Verify device exists
        if !registry.contains(&req.device_id) {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        // If device implements Stageable, call unstage()
        if let Some(stageable) = registry.get_stageable(&req.device_id) {
            stageable.unstage().await.map_err(|e| {
                Status::internal(format!(
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

    async fn execute_device_command(
        &self,
        request: Request<DeviceCommandRequest>,
    ) -> Result<Response<DeviceCommandResponse>, Status> {
        let req = request.into_inner();
        let registry = self.registry.read().await;

        // Check if device exists
        if !registry.contains(&req.device_id) {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        // Handle specific commands based on device type and command name
        match req.command.as_str() {
            #[cfg(feature = "instrument_spectra_physics")]
            "open_shutter" => {
                use crate::hardware::capabilities::ShutterControl;
                if let Some(device) = registry.get_shutter_control(&req.device_id) {
                    device
                        .open_shutter()
                        .await
                        .map_err(|e| Status::internal(format!("Failed to open shutter: {}", e)))?;
                    Ok(Response::new(DeviceCommandResponse {
                        success: true,
                        error_message: String::new(),
                        results: HashMap::new(),
                    }))
                } else {
                    Err(Status::unimplemented(format!(
                        "Device '{}' does not support ShutterControl",
                        req.device_id
                    )))
                }
            }
            #[cfg(feature = "instrument_spectra_physics")]
            "close_shutter" => {
                use crate::hardware::capabilities::ShutterControl;
                if let Some(device) = registry.get_shutter_control(&req.device_id) {
                    device
                        .close_shutter()
                        .await
                        .map_err(|e| Status::internal(format!("Failed to close shutter: {}", e)))?;
                    Ok(Response::new(DeviceCommandResponse {
                        success: true,
                        error_message: String::new(),
                        results: HashMap::new(),
                    }))
                } else {
                    Err(Status::unimplemented(format!(
                        "Device '{}' does not support ShutterControl",
                        req.device_id
                    )))
                }
            }
            #[cfg(feature = "instrument_spectra_physics")]
            "enable_emission" => {
                use crate::hardware::capabilities::EmissionControl;
                if let Some(device) = registry.get_emission_control(&req.device_id) {
                    device.enable_emission().await.map_err(|e| {
                        Status::internal(format!("Failed to enable emission: {}", e))
                    })?;
                    Ok(Response::new(DeviceCommandResponse {
                        success: true,
                        error_message: String::new(),
                        results: HashMap::new(),
                    }))
                } else {
                    Err(Status::unimplemented(format!(
                        "Device '{}' does not support EmissionControl",
                        req.device_id
                    )))
                }
            }
            #[cfg(feature = "instrument_spectra_physics")]
            "disable_emission" => {
                use crate::hardware::capabilities::EmissionControl;
                if let Some(device) = registry.get_emission_control(&req.device_id) {
                    device.disable_emission().await.map_err(|e| {
                        Status::internal(format!("Failed to disable emission: {}", e))
                    })?;
                    Ok(Response::new(DeviceCommandResponse {
                        success: true,
                        error_message: String::new(),
                        results: HashMap::new(),
                    }))
                } else {
                    Err(Status::unimplemented(format!(
                        "Device '{}' does not support EmissionControl",
                        req.device_id
                    )))
                }
            }
            _ => Err(Status::unimplemented(format!(
                "ExecuteDeviceCommand: unknown command '{}' for device '{}'",
                req.command, req.device_id
            ))),
        }
    }

    // =========================================================================
    // Observable Parameters (QCodes/ScopeFoundry pattern)
    // =========================================================================

    async fn list_parameters(
        &self,
        request: Request<ListParametersRequest>,
    ) -> Result<Response<ListParametersResponse>, Status> {
        let req = request.into_inner();
        let registry = self.registry.read().await;

        // Check if device exists
        if !registry.contains(&req.device_id) {
            return Err(Status::not_found(format!(
                "Device '{}' not found",
                req.device_id
            )));
        }

        // Get settable parameters for plugin devices
        #[cfg(feature = "tokio_serial")]
        let parameters = {
            if let Some(settables) = registry.get_settable_parameters(&req.device_id).await {
                settables
                    .into_iter()
                    .map(|s| {
                        let dtype = match s.value_type {
                            crate::hardware::plugin::schema::ValueType::Float => "float",
                            crate::hardware::plugin::schema::ValueType::Int => "int",
                            crate::hardware::plugin::schema::ValueType::String => "string",
                            crate::hardware::plugin::schema::ValueType::Enum => "enum",
                            crate::hardware::plugin::schema::ValueType::Bool => "bool",
                        };
                        ParameterDescriptor {
                            device_id: req.device_id.clone(),
                            name: s.name,
                            description: String::new(), // Not in schema
                            dtype: dtype.to_string(),
                            units: s.unit.unwrap_or_default(),
                            readable: s.get_cmd.is_some(),
                            writable: true, // Always has set_cmd
                            min_value: s.min,
                            max_value: s.max,
                            enum_values: s.options,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };

        #[cfg(not(feature = "tokio_serial"))]
        let parameters: Vec<ParameterDescriptor> = Vec::new();

        Ok(Response::new(ListParametersResponse { parameters }))
    }

    async fn get_parameter(
        &self,
        request: Request<GetParameterRequest>,
    ) -> Result<Response<ParameterValue>, Status> {
        let req = request.into_inner();

        // Try legacy Settable trait first (backwards compatibility)
        let registry = self.registry.read().await;

        if let Some(settable) = registry.get_settable(&req.device_id) {
            drop(registry); // Release lock before async operations

            // Get the parameter value
            let value = settable
                .get_value(&req.parameter_name)
                .await
                .map_err(|e| Status::internal(format!("Failed to get parameter: {}", e)))?;

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
        if let Some(params) = registry.get_parameters(&req.device_id) {
            if let Some(param) = params.get(&req.parameter_name) {
                let value = param
                    .get_json()
                    .map_err(|e| Status::internal(format!("Failed to get parameter: {}", e)))?;
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

    async fn set_parameter(
        &self,
        request: Request<SetParameterRequest>,
    ) -> Result<Response<SetParameterResponse>, Status> {
        let req = request.into_inner();

        // Try legacy Settable trait first (backwards compatibility)
        let registry = self.registry.read().await;

        if let Some(settable) = registry.get_settable(&req.device_id) {
            drop(registry); // Release lock before async operations

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
        if let Some(params) = registry.get_parameters(&req.device_id) {
            // Note: Cannot drop registry here as params borrows from it

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
                        if let Some(ref filter_device) = device_filter {
                            if &change.device_id != filter_device {
                                continue;
                            }
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
}

// Helper: fetch current device state (shared by SubscribeDeviceState)
async fn fetch_device_state(
    registry: &Arc<RwLock<DeviceRegistry>>,
    device_id: &str,
) -> Result<DeviceStateResponse, Status> {
    let (movable, readable, triggerable, frame_producer, exposure_control, exists) = {
        let registry = registry.read().await;
        (
            registry.get_movable(device_id),
            registry.get_readable(device_id),
            registry.get_triggerable(device_id),
            registry.get_frame_producer(device_id),
            registry.get_exposure_control(device_id),
            registry.contains(device_id),
        )
    };

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

    if let Some(movable) = movable {
        if let Ok(pos) = movable.position().await {
            response.position = Some(pos);
        }
    }
    if let Some(readable) = readable {
        if let Ok(val) = readable.read().await {
            response.last_reading = Some(val);
        }
    }
    if let Some(triggerable) = triggerable {
        response.armed = triggerable.is_armed().await.ok();
    }
    if let Some(frame_producer) = frame_producer {
        response.streaming = frame_producer.is_streaming().await.ok();
    }
    if let Some(exposure_ctrl) = exposure_control {
        if let Ok(seconds) = exposure_ctrl.get_exposure().await {
            response.exposure_ms = Some(seconds * 1000.0);
        }
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
fn device_info_to_proto(info: &crate::hardware::registry::DeviceInfo) -> DeviceInfo {
    DeviceInfo {
        id: info.id.clone(),
        name: info.name.clone(),
        driver_type: info.driver_type.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::registry::create_mock_registry;

    #[tokio::test]
    async fn test_list_devices() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

        let request = Request::new(ListDevicesRequest {
            capability_filter: None,
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 2);
    }

    #[tokio::test]
    async fn test_list_devices_with_filter() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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

    /// Helper to create a registry with a mock camera for frame streaming tests
    async fn create_mock_registry_with_camera() -> Result<DeviceRegistry, anyhow::Error> {
        use crate::hardware::registry::{DeviceConfig, DriverType};

        let mut registry = DeviceRegistry::new();

        // Add mock camera
        registry
            .register(DeviceConfig {
                id: "mock_camera".to_string(),
                name: "Mock Camera".to_string(),
                driver: DriverType::MockCamera,
            })
            .await?;

        Ok(registry)
    }

    #[tokio::test]
    async fn test_stream_frames_success() {
        use tokio_stream::StreamExt;

        let registry = create_mock_registry_with_camera().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

        let request = Request::new(StreamFramesRequest {
            device_id: "mock_camera".to_string(),
            include_pixel_data: Some(true),
        });
        let response = service.stream_frames(request).await.unwrap();
        let mut stream = response.into_inner();

        // Receive at least one frame
        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("timeout waiting for frame");

        assert!(frame.is_some());
        let frame_data = frame.unwrap().expect("stream item should be Ok");
        assert_eq!(frame_data.device_id, "mock_camera");
        assert!(!frame_data.pixel_data.is_empty());
    }

    #[tokio::test]
    async fn test_stream_frames_not_found() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

        let request = Request::new(StreamFramesRequest {
            device_id: "nonexistent".to_string(),
            include_pixel_data: None,
        });
        let result = service.stream_frames(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_stream_frames_wrong_capability() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

        let request = Request::new(StreamFramesRequest {
            device_id: "mock_stage".to_string(),
            include_pixel_data: None,
        });
        let result = service.stream_frames(request).await;

        assert!(result.is_err());
        // Should return NotFound since mock_stage doesn't have frame producer capability
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_stream_parameter_changes() {
        use tokio_stream::StreamExt;

        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));

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
        let service = HardwareServiceImpl::new(Arc::new(RwLock::new(registry)));
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
}
