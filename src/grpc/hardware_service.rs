//! HardwareService implementation for direct device control (bd-4x6q)
//!
//! This module provides gRPC endpoints for direct hardware manipulation,
//! bypassing the scripting layer. It connects to the DeviceRegistry for
//! capability-based access to hardware devices.

use crate::grpc::proto::{
    hardware_service_server::HardwareService, ArmRequest, ArmResponse, DeviceCommandRequest,
    DeviceCommandResponse, DeviceInfo, DeviceMetadata as ProtoDeviceMetadata, DeviceStateRequest,
    DeviceStateResponse, GetExposureRequest, GetExposureResponse, GetParameterRequest,
    ListDevicesRequest, ListDevicesResponse, ListParametersRequest, ListParametersResponse,
    MoveRequest, MoveResponse, ParameterChange, ParameterValue, PositionUpdate, ReadValueRequest,
    ReadValueResponse, SetExposureRequest, SetExposureResponse, SetParameterRequest,
    SetParameterResponse, StageDeviceRequest, StageDeviceResponse, StartStreamRequest,
    StartStreamResponse, StopMotionRequest, StopMotionResponse, StopStreamRequest,
    StopStreamResponse, StreamFramesRequest, StreamParameterChangesRequest, StreamPositionRequest,
    StreamValuesRequest, TriggerRequest, TriggerResponse, UnstageDeviceRequest,
    UnstageDeviceResponse, ValueUpdate, WaitSettledRequest, WaitSettledResponse,
};
use crate::hardware::registry::{Capability, DeviceRegistry};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

/// Hardware gRPC service implementation
///
/// Provides direct access to hardware devices through the DeviceRegistry.
/// All hardware operations are delegated to the appropriate capability traits.
pub struct HardwareServiceImpl {
    registry: Arc<RwLock<DeviceRegistry>>,
}

impl HardwareServiceImpl {
    /// Create a new HardwareService with the given device registry
    pub fn new(registry: Arc<RwLock<DeviceRegistry>>) -> Self {
        Self { registry }
    }
}

#[tonic::async_trait]
impl HardwareService for HardwareServiceImpl {
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

        match movable.move_abs(req.value).await {
            Ok(_) => {
                let final_position = movable.position().await.unwrap_or(req.value);
                Ok(Response::new(MoveResponse {
                    success: true,
                    error_message: String::new(),
                    final_position,
                }))
            }
            Err(e) => Ok(Response::new(MoveResponse {
                success: false,
                error_message: e.to_string(),
                final_position: 0.0,
            })),
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

        match movable.move_rel(req.value).await {
            Ok(_) => {
                let final_position = movable.position().await.unwrap_or(0.0);
                Ok(Response::new(MoveResponse {
                    success: true,
                    error_message: String::new(),
                    final_position,
                }))
            }
            Err(e) => Ok(Response::new(MoveResponse {
                success: false,
                error_message: e.to_string(),
                final_position: 0.0,
            })),
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
            Err(_) => {
                // Stop not supported or hardware error
                // Return success=false
                Ok(Response::new(StopMotionResponse {
                    success: false,
                    stopped_position: movable.position().await.unwrap_or(0.0),
                }))
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
            Ok(Err(_e)) => Ok(Response::new(WaitSettledResponse {
                success: false,
                settled: false,
                position: 0.0,
            })),
            Err(_) => Ok(Response::new(WaitSettledResponse {
                success: false,
                settled: false,
                position: movable.position().await.unwrap_or(0.0),
            })),
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
            Err(e) => Ok(Response::new(ReadValueResponse {
                success: false,
                error_message: e.to_string(),
                value: 0.0,
                units: String::new(),
                timestamp_ns: 0,
            })),
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
            Err(e) => Ok(Response::new(ArmResponse {
                success: false,
                error_message: e.to_string(),
                armed: false,
            })),
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
            Err(e) => Ok(Response::new(TriggerResponse {
                success: false,
                error_message: e.to_string(),
                trigger_timestamp_ns: 0,
            })),
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
            Err(e) => Ok(Response::new(SetExposureResponse {
                success: false,
                error_message: e.to_string(),
                actual_exposure_ms: 0.0,
            })),
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
            Err(e) => Ok(Response::new(StartStreamResponse {
                success: false,
                error_message: e.to_string(),
            })),
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

        // Take the frame receiver from the producer (can only be done once)
        let frame_rx = frame_producer.take_frame_receiver().await.ok_or_else(|| {
            Status::failed_precondition(format!(
                "Frame receiver for '{}' is not available (already taken or not supported)",
                device_id
            ))
        })?;

        // Create gRPC output channel
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<crate::grpc::proto::FrameData, Status>>(32);

        // Spawn background task to convert Frame → FrameData and forward to gRPC stream
        tokio::spawn(async move {
            let mut frame_rx = frame_rx;
            let mut frame_number: u32 = 0;

            while let Some(frame) = frame_rx.recv().await {
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

    async fn stage_device(
        &self,
        request: Request<StageDeviceRequest>,
    ) -> Result<Response<StageDeviceResponse>, Status> {
        let req = request.into_inner();
        // TODO: Implement device staging (prepare device for acquisition)
        // This should call a Stage trait method on the device when implemented
        Err(Status::unimplemented(format!(
            "StageDevice not yet implemented for device '{}'",
            req.device_id
        )))
    }

    async fn unstage_device(
        &self,
        request: Request<UnstageDeviceRequest>,
    ) -> Result<Response<UnstageDeviceResponse>, Status> {
        let req = request.into_inner();
        // TODO: Implement device unstaging (cleanup after acquisition)
        // This should call an Unstage trait method on the device when implemented
        Err(Status::unimplemented(format!(
            "UnstageDevice not yet implemented for device '{}'",
            req.device_id
        )))
    }

    // =========================================================================
    // Passthrough Commands (escape hatch for device-specific features)
    // =========================================================================

    async fn execute_device_command(
        &self,
        request: Request<DeviceCommandRequest>,
    ) -> Result<Response<DeviceCommandResponse>, Status> {
        let req = request.into_inner();
        // TODO: Implement passthrough command execution
        // This allows sending device-specific commands that don't fit capability traits
        Err(Status::unimplemented(format!(
            "ExecuteDeviceCommand not yet implemented for device '{}', command '{}'",
            req.device_id, req.command
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
        // TODO: Implement parameter listing
        // This should return all observable parameters for the device
        Err(Status::unimplemented(format!(
            "ListParameters not yet implemented for device '{}'",
            req.device_id
        )))
    }

    async fn get_parameter(
        &self,
        request: Request<GetParameterRequest>,
    ) -> Result<Response<ParameterValue>, Status> {
        let req = request.into_inner();
        // TODO: Implement parameter reading
        // This should return the current value of a named parameter
        Err(Status::unimplemented(format!(
            "GetParameter '{}' not yet implemented for device '{}'",
            req.parameter_name, req.device_id
        )))
    }

    async fn set_parameter(
        &self,
        request: Request<SetParameterRequest>,
    ) -> Result<Response<SetParameterResponse>, Status> {
        let req = request.into_inner();
        // TODO: Implement parameter writing
        // This should set a named parameter to a new value
        Err(Status::unimplemented(format!(
            "SetParameter '{}' not yet implemented for device '{}'",
            req.parameter_name, req.device_id
        )))
    }

    type StreamParameterChangesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ParameterChange, Status>>;

    async fn stream_parameter_changes(
        &self,
        request: Request<StreamParameterChangesRequest>,
    ) -> Result<Response<Self::StreamParameterChangesStream>, Status> {
        let req = request.into_inner();
        // TODO: Implement parameter change streaming
        // This should stream changes to observable parameters in real-time
        let device_filter = req.device_id.as_deref().unwrap_or("all devices");
        Err(Status::unimplemented(format!(
            "StreamParameterChanges not yet implemented for '{}'",
            device_filter
        )))
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
        });
        let result = service.move_absolute(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }
}
