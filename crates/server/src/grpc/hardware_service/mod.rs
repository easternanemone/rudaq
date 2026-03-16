//! HardwareService implementation for direct device control (bd-4x6q)
//!
//! This module provides gRPC endpoints for direct hardware manipulation,
//! bypassing the scripting layer. It connects to the DeviceRegistry for
//! capability-based access to hardware devices.

use crate::grpc::proto::node_state::Value as ProtoNodeValue;
use crate::grpc::{
    map_daq_error_to_status,
    proto::{
        ArmRequest,
        ArmResponse,
        CompressionType,
        DeviceCommandRequest,
        DeviceCommandResponse,
        DeviceFeature,
        DeviceInfo,
        DeviceMetadata as ProtoDeviceMetadata,
        DeviceStateRequest,
        DeviceStateResponse,
        DeviceStateSubscribeRequest,
        DeviceStateUpdate,
        FrameData,
        GetDeviceFeaturesRequest,
        GetDeviceFeaturesResponse,
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
        NodeState as ProtoNodeState,
        ObservableValue,
        ParameterChange,
        ParameterDescriptor,
        ParameterMetadata as ProtoParameterMetadata,
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
        StreamSystemStateRequest,
        StreamValuesRequest,
        StreamingMetrics,
        SystemState as ProtoSystemState,
        TriggerRequest,
        TriggerResponse,
        UnstageDeviceRequest,
        UnstageDeviceResponse,
        ValueUpdate,
        VectorValue,
        WaitSettledRequest,
        WaitSettledResponse,
        hardware_service_server::HardwareService,
    },
};
use anyhow::Error as AnyError;
use common::driver::Capability;
use common::error::DaqError;
use common::limits::{FPS_WINDOW, RPC_TIMEOUT};
use common::observable::{Observable, ParameterMetadata as CommonParameterMetadata};
use common::parameter::Parameter;
use common::state_cache::{NodeValue, SystemStateSnapshot};
use hardware::registry::DeviceRegistry;
use serde_json;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, interval};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::instrument;

mod helpers;
mod streaming;

use helpers::*;
pub(super) use streaming::StreamLimiter;
use streaming::*;

// =============================================================================
// Hardware Service Implementation
// =============================================================================

/// Hardware gRPC service implementation
///
/// Provides direct access to hardware devices through the DeviceRegistry.
/// All hardware operations are delegated to the appropriate capability traits.
pub struct HardwareServiceImpl {
    registry: Arc<DeviceRegistry>,
    /// Per-client stream limiter for DoS prevention (bd-64hu)
    stream_limiter: Arc<StreamLimiter>,
    /// Broadcast sender for parameter changes (enables real-time GUI synchronization)
    param_change_tx: tokio::sync::broadcast::Sender<ParameterChange>,
    /// Broadcast sender for system state snapshots (game loop output)
    state_broadcast_tx: Option<tokio::sync::broadcast::Sender<SystemStateSnapshot>>,
    /// Optional DB access for offline feature queries (bd-mmjc)
    #[cfg(feature = "db-surreal")]
    db: Option<Arc<db::DaqDb>>,
}

impl HardwareServiceImpl {
    async fn await_with_timeout<F, T>(&self, operation: &str, fut: F) -> Result<T, Status>
    where
        F: Future<Output = Result<T, AnyError>> + Send,
        T: Send,
    {
        match tokio::time::timeout(RPC_TIMEOUT, fut).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(map_anyhow_error_to_status(err)),
            Err(_) => Err(Status::deadline_exceeded(format!(
                "{} timed out after {:?}",
                operation, RPC_TIMEOUT
            ))),
        }
    }

    /// Execute a device I/O operation with timeout and health state reporting (bd-zs0l).
    ///
    /// Wraps `await_with_timeout` and reports success/failure to the device health
    /// tracker so the supervisor can detect degraded or faulted devices.
    ///
    /// Only device/transport errors (Internal, Unavailable, DeadlineExceeded) count
    /// as health failures. Client-side errors (InvalidArgument, NotFound, etc.) are
    /// passed through without affecting device health state.
    async fn await_with_health_reporting<F, T>(
        &self,
        device_id: &str,
        operation: &str,
        fut: F,
    ) -> Result<T, Status>
    where
        F: Future<Output = Result<T, AnyError>> + Send,
        T: Send,
    {
        match self.await_with_timeout(operation, fut).await {
            Ok(value) => {
                self.registry.report_device_success(device_id);
                Ok(value)
            }
            Err(status) => {
                if Self::is_device_error(&status) {
                    self.registry
                        .report_device_failure(device_id, status.message());
                }
                Err(status)
            }
        }
    }

    /// Returns true if the gRPC status indicates a device/transport failure
    /// rather than a client-side error.
    fn is_device_error(status: &Status) -> bool {
        matches!(
            status.code(),
            tonic::Code::Internal
                | tonic::Code::Unavailable
                | tonic::Code::DeadlineExceeded
                | tonic::Code::FailedPrecondition
        )
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
            stream_limiter: Arc::new(StreamLimiter::new()),
            param_change_tx,
            state_broadcast_tx: None,
            #[cfg(feature = "db-surreal")]
            db: None,
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
            stream_limiter: Arc::new(StreamLimiter::new()),
            param_change_tx,
            state_broadcast_tx: None,
            #[cfg(feature = "db-surreal")]
            db: None,
        }
    }

    /// Attach a system state broadcast sender (from the game loop) for StreamSystemState.
    pub fn with_state_broadcast(
        mut self,
        tx: tokio::sync::broadcast::Sender<SystemStateSnapshot>,
    ) -> Self {
        self.state_broadcast_tx = Some(tx);
        self
    }

    /// Attach a SurrealDB instance for offline device feature queries (bd-mmjc).
    #[cfg(feature = "db-surreal")]
    pub fn with_db(mut self, db: Option<db::DaqDb>) -> Self {
        self.db = db.map(Arc::new);
        self
    }

    /// Get a clone of the parameter change broadcast sender for external notification
    pub fn param_change_sender(&self) -> tokio::sync::broadcast::Sender<ParameterChange> {
        self.param_change_tx.clone()
    }
}

/// Helper macro to reduce boilerplate for capability lookups
///
/// Usage: require_capability!(self, get_movable, &device_id, "not movable")
///
/// Expands to:
/// ```ignore
/// let capability = self.registry.$getter($device_id);
/// let capability = capability.ok_or_else(|| {
///     Status::not_found(format!("Device '{}' {}",
///         $device_id, $capability_desc))
/// })?;
/// ```
macro_rules! require_capability {
    ($self:expr, $getter:ident, $device_id:expr, $capability_desc:literal) => {{
        let capability = $self.registry.$getter($device_id);
        capability.ok_or_else(|| {
            Status::not_found(format!("Device '{}' {}", $device_id, $capability_desc))
        })?
    }};
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
                .map(|info| {
                    let health = self.registry.get_device_health(&info.id);
                    helpers::device_info_to_proto_with_health(&info, health.as_ref())
                })
                .collect()
        } else {
            // Return all devices
            self.registry
                .list_devices()
                .iter()
                .map(|info| {
                    let health = self.registry.get_device_health(&info.id);
                    helpers::device_info_to_proto_with_health(info, health.as_ref())
                })
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

        // Populate health fields from registry (bd-vgrj)
        let health = self.registry.get_device_health(&req.device_id);
        let (health_status, consecutive_failures, restart_attempts, last_error, is_faulted) =
            if let Some(ref h) = health {
                (
                    helpers::device_health_to_proto(h.health),
                    h.consecutive_failures,
                    h.restart_attempts,
                    h.last_error.clone().unwrap_or_default(),
                    h.health == common::health::DeviceHealth::Faulted,
                )
            } else {
                (
                    helpers::device_health_to_proto(common::health::DeviceHealth::Healthy),
                    0,
                    0,
                    String::new(),
                    false,
                )
            };

        let mut response = DeviceStateResponse {
            device_id: req.device_id.clone(),
            online: !is_faulted,
            position: None,
            last_reading: None,
            armed: None,
            streaming: None,
            exposure_ms: None,
            health_status,
            consecutive_failures,
            restart_attempts,
            last_error,
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
            // Not critical if read fails — device state query is best-effort
            if let Ok(val) = readable.read().await {
                response.last_reading = Some(val);
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
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // SAFETY: value is validated/bounded before cast
        let interval_ms = if req.max_rate_hz > 0 {
            (1000.0 / (f64::from(req.max_rate_hz))).max(10.0) as u64
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
                for device_id in &device_ids {
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
                        .copied()
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
        let movable = require_capability!(
            self,
            get_movable,
            &req.device_id,
            "not found or not movable"
        );

        // Parameter bounds validation (bd-izdj.3)
        if let Some(meta) = self
            .registry
            .get_parameter_metadata(&req.device_id, "position")
        {
            // Validate against min_position if set
            if let Some(min) = meta.min_value
                && req.value < min
            {
                return Err(Status::invalid_argument(format!(
                    "Target position {} is below minimum {}",
                    req.value, min
                )));
            }
            // Validate against max_position if set
            if let Some(max) = meta.max_value
                && req.value > max
            {
                return Err(Status::invalid_argument(format!(
                    "Target position {} is above maximum {}",
                    req.value, max
                )));
            }
        }

        self.await_with_health_reporting(&req.device_id, "move_abs", movable.move_abs(req.value))
            .await?;

        let (final_position, settled) = if req.wait_for_completion.unwrap_or(false) {
            if let Some(timeout_ms) = req.timeout_ms {
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(timeout_ms)),
                    movable.wait_settled(),
                )
                .await
                {
                    Ok(Ok(())) => {
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
                self.await_with_health_reporting(
                    &req.device_id,
                    "wait_settled",
                    movable.wait_settled(),
                )
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
        let movable = require_capability!(
            self,
            get_movable,
            &req.device_id,
            "not found or not movable"
        );

        // Parameter bounds validation (bd-izdj.3)
        // For relative moves, we need to know current position to validate bounds
        if let Some(info) = self.registry.get_device_info(&req.device_id) {
            // Only check if bounds are defined
            if info.metadata.min_position.is_some() || info.metadata.max_position.is_some() {
                // We must get current position to validate target
                // Note: This adds a read operation before the move, but safety is priority
                match movable.position().await {
                    Ok(current_pos) => {
                        let target = current_pos + req.value;

                        if let Some(min) = info.metadata.min_position
                            && target < min
                        {
                            return Err(Status::invalid_argument(format!(
                                "Relative move to {} is below minimum {}",
                                target, min
                            )));
                        }

                        if let Some(max) = info.metadata.max_position
                            && target > max
                        {
                            return Err(Status::invalid_argument(format!(
                                "Relative move to {} is above maximum {}",
                                target, max
                            )));
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            device_id = %req.device_id,
                            error = %e,
                            "Failed to read current position for bounds validation"
                        );
                        return Err(Status::unavailable(format!(
                            "Cannot validate relative move: failed to read current position: {}",
                            e
                        )));
                    }
                }
            }
        }

        self.await_with_health_reporting(&req.device_id, "move_rel", movable.move_rel(req.value))
            .await?;

        let (final_position, settled) = if req.wait_for_completion.unwrap_or(false) {
            if let Some(timeout_ms) = req.timeout_ms {
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(timeout_ms)),
                    movable.wait_settled(),
                )
                .await
                {
                    Ok(Ok(())) => {
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
                self.await_with_health_reporting(
                    &req.device_id,
                    "wait_settled",
                    movable.wait_settled(),
                )
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
        let movable = require_capability!(
            self,
            get_movable,
            &req.device_id,
            "not found or not movable"
        );

        self.await_with_health_reporting(&req.device_id, "stop_motion", movable.stop())
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
        let movable = require_capability!(
            self,
            get_movable,
            &req.device_id,
            "not found or not movable"
        );

        if let Some(timeout_ms) = req.timeout_ms {
            match tokio::time::timeout(
                Duration::from_millis(u64::from(timeout_ms)),
                movable.wait_settled(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(map_hardware_error_to_status(&e.to_string())),
                Err(_) => {
                    return Err(Status::deadline_exceeded(format!(
                        "Wait settled operation timed out for device '{}'",
                        req.device_id
                    )));
                }
            }
        } else {
            self.await_with_health_reporting(
                &req.device_id,
                "wait_settled",
                movable.wait_settled(),
            )
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
            let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(rate_hz));
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

                    #[allow(clippy::cast_possible_truncation)]
                    // SAFETY: value is bounded and fits in target type
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
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    async fn read_value(
        &self,
        request: Request<ReadValueRequest>,
    ) -> Result<Response<ReadValueResponse>, Status> {
        let req = request.into_inner();
        tracing::debug!("read_value called for device_id={}", req.device_id);

        // Extract Arc and metadata without lock before awaiting hardware
        let readable = require_capability!(
            self,
            get_readable,
            &req.device_id,
            "not found or not readable"
        );
        let units = self
            .registry
            .get_device_info(&req.device_id)
            .and_then(|info| info.metadata.measurement_units.clone())
            .unwrap_or_default();

        let value = self
            .await_with_health_reporting(&req.device_id, "read_value", readable.read())
            .await?;

        tracing::debug!(
            "read_value response: device_id={}, value={}, units='{}'",
            req.device_id,
            value,
            units
        );

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
            let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(rate_hz));
            let mut ticker = tokio::time::interval(interval);

            // Mark device as actively streaming (prevents hot-swap reconfiguration).
            registry
                .set_measurement_lock(&device_id, common::capabilities::MeasurementLock::Measuring);

            loop {
                ticker.tick().await;

                // Get readable and metadata directly from registry
                let readable = registry.get_readable(&device_id);
                let units = registry
                    .get_device_info(&device_id)
                    .and_then(|info| info.metadata.measurement_units.clone())
                    .unwrap_or_default();

                if let Some(readable) = readable {
                    match readable.read().await {
                        Ok(value) => {
                            registry.report_device_success(&device_id);
                            #[allow(clippy::cast_possible_truncation)]
                            // SAFETY: value is bounded and fits in target type
                            let update = ValueUpdate {
                                device_id: device_id.clone(),
                                value,
                                units,
                                timestamp_ns: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos()
                                    as u64,
                            };

                            if tx.send(Ok(update)).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            registry.report_device_failure(
                                &device_id,
                                format!("stream_values read failed: {}", e),
                            );
                        }
                    }
                } else {
                    break;
                }
            }

            // Release lock when streaming ends (client disconnect or device removed).
            registry.set_measurement_lock(&device_id, common::capabilities::MeasurementLock::Idle);
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
        let triggerable = require_capability!(
            self,
            get_triggerable,
            &req.device_id,
            "not found or not triggerable"
        );

        match triggerable.arm().await {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(ArmResponse {
                    success: true,
                    error_message: String::new(),
                    armed: true,
                }))
            }
            Err(e) => {
                let err_msg = e.to_string();
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
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
        let triggerable = require_capability!(
            self,
            get_triggerable,
            &req.device_id,
            "not found or not triggerable"
        );

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        match triggerable.trigger().await {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(TriggerResponse {
                    success: true,
                    error_message: String::new(),
                    trigger_timestamp_ns: timestamp_ns,
                }))
            }
            Err(e) => {
                let err_msg = e.to_string();
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
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
        let exposure_ctrl = require_capability!(
            self,
            get_exposure_control,
            &req.device_id,
            "not found or has no exposure control"
        );

        // Convert ms to seconds for the trait API
        let exposure_seconds = req.exposure_ms / 1000.0;

        match exposure_ctrl.set_exposure(exposure_seconds).await {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
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
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
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
        let exposure_ctrl = require_capability!(
            self,
            get_exposure_control,
            &req.device_id,
            "not found or has no exposure control"
        );

        // Convert seconds to ms for response
        match exposure_ctrl.get_exposure().await {
            Ok(seconds) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(GetExposureResponse {
                    exposure_ms: seconds * 1000.0,
                }))
            }
            Err(e) => {
                let err_msg = format!("Failed to get exposure: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
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

        let shutter_ctrl = require_capability!(
            self,
            get_shutter_control,
            &req.device_id,
            "not found or has no shutter control"
        );

        let open = req.open;
        match if open {
            shutter_ctrl.open_shutter().await
        } else {
            shutter_ctrl.close_shutter().await
        } {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(SetShutterResponse {
                    success: true,
                    error_message: String::new(),
                    is_open: open,
                }))
            }
            Err(e) => {
                let err_msg = format!("Failed to set shutter: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
        }
    }

    async fn get_shutter(
        &self,
        request: Request<GetShutterRequest>,
    ) -> Result<Response<GetShutterResponse>, Status> {
        let req = request.into_inner();

        let shutter_ctrl = require_capability!(
            self,
            get_shutter_control,
            &req.device_id,
            "not found or has no shutter control"
        );

        match shutter_ctrl.is_shutter_open().await {
            Ok(is_open) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(GetShutterResponse { is_open }))
            }
            Err(e) => {
                let err_msg = format!("Failed to get shutter state: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
        }
    }

    #[instrument(skip(self, request), fields(method = "set_wavelength"))]
    async fn set_wavelength(
        &self,
        request: Request<SetWavelengthRequest>,
    ) -> Result<Response<SetWavelengthResponse>, Status> {
        let req = request.into_inner();

        let wavelength_ctrl = require_capability!(
            self,
            get_wavelength_tunable,
            &req.device_id,
            "not found or has no wavelength control"
        );

        let requested_nm = req.wavelength_nm;
        match wavelength_ctrl.set_wavelength(requested_nm).await {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(SetWavelengthResponse {
                    success: true,
                    error_message: String::new(),
                    actual_wavelength_nm: requested_nm,
                }))
            }
            Err(e) => {
                let err_msg = format!("Failed to set wavelength: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
        }
    }

    async fn get_wavelength(
        &self,
        request: Request<GetWavelengthRequest>,
    ) -> Result<Response<GetWavelengthResponse>, Status> {
        let req = request.into_inner();

        let wavelength_ctrl = require_capability!(
            self,
            get_wavelength_tunable,
            &req.device_id,
            "not found or has no wavelength control"
        );

        match wavelength_ctrl.get_wavelength().await {
            Ok(nm) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(GetWavelengthResponse { wavelength_nm: nm }))
            }
            Err(e) => {
                let err_msg = format!("Failed to get wavelength: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
        }
    }

    #[instrument(skip(self, request), fields(method = "set_emission"))]
    async fn set_emission(
        &self,
        request: Request<SetEmissionRequest>,
    ) -> Result<Response<SetEmissionResponse>, Status> {
        let req = request.into_inner();
        log::info!(
            ">>> set_emission RPC called: device={}, enabled={}",
            req.device_id,
            req.enabled
        );

        let emission_ctrl = require_capability!(
            self,
            get_emission_control,
            &req.device_id,
            "not found or has no emission control"
        );

        let enabled = req.enabled;
        match if enabled {
            emission_ctrl.enable_emission().await
        } else {
            emission_ctrl.disable_emission().await
        } {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(SetEmissionResponse {
                    success: true,
                    error_message: String::new(),
                    is_enabled: enabled,
                }))
            }
            Err(e) => {
                let err_msg = format!("Failed to set emission: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
        }
    }

    #[instrument(skip(self, request), fields(method = "get_emission"))]
    async fn get_emission(
        &self,
        request: Request<GetEmissionRequest>,
    ) -> Result<Response<GetEmissionResponse>, Status> {
        let req = request.into_inner();
        log::info!(">>> get_emission RPC called: device={}", req.device_id);

        let emission_ctrl = require_capability!(
            self,
            get_emission_control,
            &req.device_id,
            "not found or has no emission control"
        );

        log::info!(">>> get_emission: calling is_emission_enabled()...");
        match emission_ctrl.is_emission_enabled().await {
            Ok(is_enabled) => {
                self.registry.report_device_success(&req.device_id);
                log::info!(">>> get_emission: is_enabled={}", is_enabled);
                Ok(Response::new(GetEmissionResponse { is_enabled }))
            }
            Err(e) => {
                let err_msg = format!("Failed to get emission state: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
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
        let frame_producer = require_capability!(
            self,
            get_frame_producer,
            &req.device_id,
            "not found or not a frame producer"
        );

        // Use frame_count from request (0 or None = continuous)
        let frame_limit = req.frame_count.filter(|&n| n > 0);

        match frame_producer.start_stream_finite(frame_limit).await {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                Ok(Response::new(StartStreamResponse {
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                let err_msg = e.to_string();
                // Idempotent: treat "already streaming" as success
                if err_msg.to_lowercase().contains("already streaming") {
                    self.registry.report_device_success(&req.device_id);
                    tracing::info!(device_id = %req.device_id, "Device already streaming (idempotent success)");
                    Ok(Response::new(StartStreamResponse {
                        success: true,
                        error_message: "Already streaming".to_string(),
                    }))
                } else {
                    self.registry
                        .report_device_failure(&req.device_id, &err_msg);
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
        tracing::debug!(device_id = %req.device_id, "stop_stream called");

        // Extract Arc without lock before awaiting hardware
        let frame_producer = require_capability!(
            self,
            get_frame_producer,
            &req.device_id,
            "not found or not a frame producer"
        );

        match frame_producer.stop_stream().await {
            Ok(()) => {
                self.registry.report_device_success(&req.device_id);
                // Get frame count from device
                let frames_captured = frame_producer.frame_count();
                Ok(Response::new(StopStreamResponse {
                    success: true,
                    frames_captured,
                }))
            }
            Err(e) => {
                let err_msg = format!("Failed to stop stream: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                Err(map_hardware_error_to_status(&err_msg))
            }
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
    ///
    /// Per-client rate limiting (bd-64hu): Each client IP is limited to
    /// MAX_STREAMS_PER_CLIENT concurrent frame streams to prevent DoS.
    async fn stream_frames(
        &self,
        request: Request<StreamFramesRequest>,
    ) -> Result<Response<Self::StreamFramesStream>, Status> {
        // Extract client IP for rate limiting (bd-64hu)
        let client_ip = request
            .remote_addr()
            .map(|addr| addr.ip())
            .unwrap_or_else(|| IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));

        // Check per-client stream limit (bd-64hu)
        let stream_slot = self.stream_limiter.try_acquire_guard(client_ip)?;

        let req = request.into_inner();
        let device_id = req.device_id.clone();
        let max_fps = req.max_fps;
        let quality = req.quality();

        // Get frame producer
        let frame_producer = require_capability!(
            self,
            get_frame_producer,
            &device_id,
            "not found or not a frame producer"
        );

        // Check if device supports observers (bd-0dax.6.3)
        if !frame_producer.supports_observers() {
            return Err(Status::unavailable(format!(
                "Device '{}' does not support frame observers. \
                 The driver must implement register_observer() for tap-based streaming.",
                device_id
            )));
        }

        // Channel capacity constants (bd-rgnx.11: increased for streaming throughput)
        // Observer channel: bounded to handle backpressure in on_frame()
        // Increased from 16 to 32 to absorb burst frames and reduce drop rate
        const OBSERVER_CHANNEL_CAPACITY: usize = 32;
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
            Some(Duration::from_secs_f64(1.0 / f64::from(max_fps)))
        } else {
            None
        };

        // Spawn task to forward frames from observer channel to gRPC stream
        // Use Arc<str> to avoid per-frame String clones in the hot path (bd-rgnx.11)
        let device_id_arc: Arc<str> = Arc::from(device_id.as_str());
        let device_id_clone = device_id.clone();
        let frame_producer_clone = frame_producer.clone();
        let registry_clone = self.registry.clone();

        // Dedicated compression thread: receives ObserverFramePacket, compresses with
        // buffer reuse, and sends FrameData to the async forwarding task.
        // Eliminates per-frame spawn_blocking overhead (~50-200μs) and per-frame
        // compression buffer allocation.
        let (compress_tx, mut compress_rx) = tokio::sync::mpsc::channel::<(
            ObserverFramePacket,
            StreamingMetrics,
        )>(GRPC_CHANNEL_CAPACITY);
        let device_id_for_compressor = Arc::clone(&device_id_arc);
        let (compressed_tx, mut compressed_rx) =
            tokio::sync::mpsc::channel::<(FrameData, usize, usize)>(GRPC_CHANNEL_CAPACITY);

        std::thread::Builder::new()
            .name(format!("lz4-compress-{device_id}"))
            .spawn({
                let compressed_tx = compressed_tx;
                move || {
                    let mut compress_buf = Vec::new();

                    while let Some((packet, metrics)) = compress_rx.blocking_recv() {
                        let mut frame_data = FrameData {
                            device_id: device_id_for_compressor.to_string(),
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
                            gain_mode: None,
                            readout_speed: None,
                            trigger_mode: None,
                            binning_x: packet.binning.map(|(x, _)| u32::from(x)),
                            binning_y: packet.binning.map(|(_, y)| u32::from(y)),
                            metadata: HashMap::new(),
                            metrics: Some(metrics),
                            compression: CompressionType::CompressionNone as i32,
                            uncompressed_size: 0,
                        };

                        let uncompressed_size = frame_data.data.len();
                        crate::grpc::compression::compress_frame_into(
                            &mut frame_data,
                            &mut compress_buf,
                        );
                        let compressed_size = frame_data.data.len();

                        if compressed_tx
                            .blocking_send((frame_data, uncompressed_size, compressed_size))
                            .is_err()
                        {
                            break; // Forwarding task dropped — exit
                        }
                    }
                }
            })
            .expect("failed to spawn LZ4 compression thread");

        tokio::spawn(async move {
            // Hold the per-client stream slot for the lifetime of this forwarding task.
            // Dropping the guard releases the slot on every exit path (including panic unwind).
            let stream_slot_guard = stream_slot;
            // Initialize to allow first frame through immediately
            let mut last_frame_time = match min_interval {
                Some(interval) => std::time::Instant::now().checked_sub(interval).unwrap(),
                None => std::time::Instant::now(),
            };
            let mut frames_sent = 0u64;
            let mut frames_dropped = 0u64;
            let mut fps_window: VecDeque<std::time::Instant> = VecDeque::new();
            let mut avg_latency_ms = 0.0f64;
            let mut latency_samples = 0u64;
            // Track whether we have reported success this session.
            // Reporting on the first delivered frame resets any accumulated
            // failure count from a previous error, keeping the consecutive-
            // failure semantics of the health tracker meaningful.
            let mut success_reported = false;

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
                // Multiplex: read from observer (new frames), compression thread (compressed
                // frames ready to send), and watch for gRPC client disconnect.
                tokio::select! {
                    // Handle compressed frames ready to send to gRPC client
                    compressed_result = compressed_rx.recv() => {
                        match compressed_result {
                            Some((frame_data, uncompressed_size, compressed_size)) => {
                                // Log early frame sends
                                if frames_sent < 10 {
                                    tracing::info!(
                                        device_id = %device_id_clone,
                                        frame = frames_sent + 1,
                                        frame_number = frame_data.frame_number,
                                        bytes = frame_data.data.len(),
                                        queue_capacity = grpc_tx.capacity(),
                                        "About to send frame to gRPC client (early frame debug)"
                                    );
                                }

                                // Send to gRPC client
                                if grpc_tx.send(Ok(frame_data)).await.is_err() {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        frames_sent = frames_sent,
                                        "Client disconnected from frame stream - gRPC send failed"
                                    );

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

                                // On the first successfully delivered frame, reset the device's
                                // consecutive-failure counter so that transient streaming errors
                                // do not accumulate toward Faulted once the stream recovers.
                                if !success_reported {
                                    registry_clone.report_device_success(&device_id_clone);
                                    success_reported = true;
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
                                    #[allow(clippy::cast_precision_loss)]
                                    // SAFETY: precision loss acceptable for metrics/display
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
                                // Compression thread exited
                                tracing::info!(
                                    device_id = %device_id_clone,
                                    frames_sent = frames_sent,
                                    "Compression thread exited"
                                );
                                exit_reason = "compressor_closed";
                                break;
                            }
                        }
                    }
                    // Handle new frames from the observer channel
                    next_packet = observer_rx.recv() => {
                        match next_packet {
                            Some(mut packet) => {
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

                                // Validate frame dimensions and normalize pixel format (bd-7rk0, bd-q2n6)
                                let pixel_count = (packet.width as usize)
                                    .saturating_mul(packet.height as usize);
                                let expected_u16 = pixel_count.saturating_mul(2);
                                let expected_mono12_packed = pixel_count.saturating_mul(3) / 2;

                                if packet.data.len() == expected_u16 {
                                    // Mono16 or 12-bit-in-16-bit container — normal path
                                } else if packet.data.len() == expected_mono12_packed
                                    || packet.data.len() == expected_mono12_packed + (pixel_count % 2)
                                {
                                    // Mono12Packed: 3 bytes per 2 pixels → unpack to u16
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        actual_size = packet.data.len(),
                                        "Unpacking Mono12 frame to u16 (camera using packed pixel encoding)"
                                    );
                                    let mut unpacked = Vec::with_capacity(expected_u16);
                                    let src = &packet.data;
                                    let mut i = 0;
                                    while i + 2 < src.len() {
                                        // Mono12Packed: [P0_hi, P0_lo:P1_lo, P1_hi]
                                        let p0 = (u16::from(src[i]) << 4) | u16::from(src[i + 1] & 0x0F);
                                        let p1 = (u16::from(src[i + 2]) << 4) | u16::from(src[i + 1] >> 4);
                                        unpacked.extend_from_slice(&p0.to_le_bytes());
                                        unpacked.extend_from_slice(&p1.to_le_bytes());
                                        i += 3;
                                    }
                                    unpacked.truncate(expected_u16);
                                    packet.data = unpacked;
                                    packet.bit_depth = 16;
                                } else {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        width = packet.width,
                                        height = packet.height,
                                        bit_depth = packet.bit_depth,
                                        actual_size = packet.data.len(),
                                        expected_u16,
                                        expected_mono12_packed,
                                        "Frame data size does not match any known pixel format, skipping"
                                    );
                                    frames_dropped = frames_dropped.saturating_add(1);
                                    continue;
                                }

                                // Update FPS tracking
                                let now_instant = std::time::Instant::now();
                                fps_window.push_back(now_instant);
                                while let Some(front) = fps_window.front() {
                                    if now_instant.duration_since(*front) > FPS_WINDOW {
                                        fps_window.pop_front();
                                    } else {
                                        break;
                                    }
                                }
                                #[allow(clippy::cast_precision_loss)]
                                // SAFETY: precision loss acceptable for metrics/display
                                let current_fps = fps_window.len() as f64;

                                // Update latency tracking
                                if packet.timestamp_ns > 0 {
                                    #[allow(clippy::cast_precision_loss)]
                                    // SAFETY: precision loss acceptable for metrics/display
                                    let latency_ms =
                                        now_ns().saturating_sub(packet.timestamp_ns) as f64
                                            / 1_000_000.0;
                                    latency_samples = latency_samples.saturating_add(1);
                                    #[allow(clippy::cast_precision_loss)]
                                    // SAFETY: precision loss acceptable for running average
                                    let samples_f64 = latency_samples as f64;
                                    avg_latency_ms += (latency_ms - avg_latency_ms) / samples_f64;
                                }

                                // Increment before building metrics (matches original behavior
                                // where frames_sent reflects "frames submitted for sending")
                                frames_sent = frames_sent.saturating_add(1);

                                let metrics = StreamingMetrics {
                                    current_fps,
                                    frames_sent,
                                    frames_dropped,
                                    avg_latency_ms,
                                };

                                // Non-blocking send: if the compressor is backlogged
                                // we drop this frame rather than awaiting, which would
                                // prevent the select! from draining compressed_rx and
                                // cause a deadlock between the two bounded channels.
                                match compress_tx.try_send((packet, metrics)) {
                                    Ok(()) => {}
                                    Err(
                                        tokio::sync::mpsc::error::TrySendError::Full(_),
                                    ) => {
                                        frames_dropped =
                                            frames_dropped.saturating_add(1);
                                        tracing::debug!(
                                            device_id = %device_id_clone,
                                            frames_dropped,
                                            "Compressor channel full; dropping frame"
                                        );
                                    }
                                    Err(
                                        tokio::sync::mpsc::error::TrySendError::Closed(
                                            _,
                                        ),
                                    ) => {
                                        tracing::warn!(
                                            device_id = %device_id_clone,
                                            "Compression thread channel closed"
                                        );
                                        exit_reason = "compressor_closed";
                                        break;
                                    }
                                }
                            }
                            None => {
                                // Observer channel closed - producer stopped or observer was dropped.
                                // Check if this was due to a hardware error (e.g. USB/PCIe disconnect)
                                // so the supervisor can schedule an automatic reconnection attempt.
                                if frame_producer_clone.has_acquisition_error() {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        frames_sent = frames_sent,
                                        "Observer channel closed due to acquisition error — \
                                         reporting failure for supervisor reconnection"
                                    );
                                    registry_clone.report_device_failure(
                                        &device_id_clone,
                                        "acquisition loop exited with hardware error",
                                    );
                                } else {
                                    tracing::info!(
                                        device_id = %device_id_clone,
                                        frames_sent = frames_sent,
                                        "Observer channel closed - producer stopped streaming"
                                    );
                                }

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
                    // Handle gRPC client disconnect
                    () = grpc_tx.closed() => {
                        tracing::info!(
                            device_id = %device_id_clone,
                            frames_sent = frames_sent,
                            "gRPC frame stream receiver dropped; stopping forwarding task"
                        );

                        if let Err(e) = frame_producer_clone
                            .unregister_observer(observer_handle)
                            .await
                        {
                            tracing::debug!(
                                device_id = %device_id_clone,
                                observer_handle = observer_handle.id(),
                                error = %e,
                                "Failed to unregister observer after gRPC receiver drop (may already be unregistered)"
                            );
                        } else {
                            tracing::info!(
                                device_id = %device_id_clone,
                                observer_handle = observer_handle.id(),
                                "Unregistered observer after gRPC receiver drop"
                            );
                        }

                        exit_reason = "grpc_receiver_dropped";
                        break;
                    }
                }
            }
            // Release stream slot (bd-64hu)
            // Dropping compress_tx closes the compression thread channel, causing it to exit
            drop(compress_tx);
            drop(stream_slot_guard);

            // Final summary log
            tracing::info!(
                device_id = %device_id_clone,
                exit_reason = exit_reason,
                frames_sent = frames_sent,
                frames_dropped = frames_dropped,
                client_ip = %client_ip,
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
                let err_msg = format!("Failed to stage device '{}': {}", req.device_id, e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                map_hardware_error_to_status(&err_msg)
            })?;
            self.registry.report_device_success(&req.device_id);
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
                let err_msg = format!("Failed to unstage device '{}': {}", req.device_id, e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                map_hardware_error_to_status(&err_msg)
            })?;
            self.registry.report_device_success(&req.device_id);
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
                    let err_msg = format!("Command execution failed: {}", e);
                    self.registry
                        .report_device_failure(&req.device_id, &err_msg);
                    map_hardware_error_to_status(&err_msg)
                })?;

            self.registry.report_device_success(&req.device_id);
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
                    let observable_metadata = param.metadata();
                    // Use live metadata from the parameter itself. Registry metadata is a
                    // registration-time snapshot and can be stale for dynamic choices.
                    let live_metadata = CommonParameterMetadata::from(&observable_metadata);

                    // Use introspectable dtype from metadata if available,
                    // otherwise infer from current value (best-effort fallback)
                    let dtype = if !live_metadata.dtype.is_empty() {
                        live_metadata.dtype.clone()
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

                    let proto_metadata = proto_parameter_metadata(&live_metadata);

                    parameters.push(ParameterDescriptor {
                        device_id: req.device_id.clone(),
                        name: observable_metadata.name.clone(),
                        description: live_metadata.description.clone().unwrap_or_default(),
                        dtype,
                        units: live_metadata.units.clone().unwrap_or_default(),
                        readable: true,
                        writable: !live_metadata.read_only,
                        min_value: live_metadata.min_value,
                        max_value: live_metadata.max_value,
                        enum_values: live_metadata.enum_values.clone(),
                        metadata: Some(proto_metadata),
                        group_name: live_metadata.group_name.clone(),
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

        // New path - use Parameterized trait first (synchronous cache)
        if let Some(parameterized) = self.registry.get_parameterized(&req.device_id) {
            let params = parameterized.parameters();
            if let Some(param) = params.get(&req.parameter_name) {
                let value = param.get_json().map_err(|e| {
                    map_hardware_error_to_status(&format!("Failed to get parameter: {}", e))
                })?;
                let units = CommonParameterMetadata::from(&param.metadata())
                    .units
                    .unwrap_or_default();
                #[allow(clippy::cast_possible_truncation)]
                // SAFETY: value is bounded and fits in target type
                let timestamp_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);

                return Ok(Response::new(ParameterValue {
                    device_id: req.device_id,
                    name: req.parameter_name,
                    value: value.to_string(),
                    units,
                    timestamp_ns,
                }));
            }
        }

        // Try legacy Settable trait as a fallback (asynchronous hardware read)
        if let Some(settable) = self.registry.get_settable(&req.device_id) {
            // Get the parameter value
            let value = settable.get_value(&req.parameter_name).await.map_err(|e| {
                let err_msg = format!("Failed to get parameter: {}", e);
                self.registry
                    .report_device_failure(&req.device_id, &err_msg);
                map_hardware_error_to_status(&err_msg)
            })?;
            self.registry.report_device_success(&req.device_id);
            let units = self
                .registry
                .get_parameter_metadata(&req.device_id, &req.parameter_name)
                .and_then(|meta| meta.units)
                .unwrap_or_default();

            // Get timestamp
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: value is bounded and fits in target type
            let timestamp_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);

            return Ok(Response::new(ParameterValue {
                device_id: req.device_id,
                name: req.parameter_name,
                value: value.to_string(),
                units,
                timestamp_ns,
            }));
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
        tracing::debug!(
            device_id = %req.device_id,
            param = %req.parameter_name,
            value = %req.value,
            "set_parameter called"
        );

        // Try legacy Settable trait first (backwards compatibility)
        if let Some(settable) = self.registry.get_settable(&req.device_id) {
            tracing::debug!(device_id = %req.device_id, param = %req.parameter_name, "set_parameter: using Settable path");
            let metadata = self
                .registry
                .get_parameter_metadata(&req.device_id, &req.parameter_name);
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

            validate_parameter_value(&req.parameter_name, metadata.as_ref(), &json_value)?;

            // Set the parameter
            settable
                .set_value(&req.parameter_name, json_value)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to set parameter: {e}");
                    self.registry
                        .report_device_failure(&req.device_id, &err_msg);
                    map_hardware_error_to_status(&err_msg)
                })?;
            self.registry.report_device_success(&req.device_id);

            // Read back the actual value
            let actual_value = settable
                .get_value(&req.parameter_name)
                .await
                .map(|v| v.to_string())
                .unwrap_or_else(|_| req.value.clone());

            let units = metadata
                .as_ref()
                .and_then(|meta| meta.units.clone())
                .unwrap_or_default();

            tracing::debug!(
                device_id = %req.device_id,
                param = %req.parameter_name,
                %old_value,
                %actual_value,
                "set_parameter: Settable path succeeded"
            );

            // Broadcast parameter change notification (ignore send errors - no subscribers is ok)
            let _ = self.param_change_tx.send(ParameterChange {
                device_id: req.device_id.clone(),
                name: req.parameter_name.clone(),
                old_value,
                new_value: actual_value.clone(),
                units,
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
            tracing::debug!(device_id = %req.device_id, param = %req.parameter_name, "set_parameter: using Parameterized path");
            let params = parameterized.parameters();

            if let Some(param) = params.get(&req.parameter_name) {
                let metadata = CommonParameterMetadata::from(&param.metadata());
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

                validate_parameter_value(&req.parameter_name, Some(&metadata), &json_value)?;

                // Set the parameter (synchronous call, no await needed)
                param
                    .set_json(json_value)
                    .map_err(map_anyhow_error_to_status)?;

                let actual_value = param
                    .get_json()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| req.value.clone());

                let units = metadata.units.clone().unwrap_or_default();

                tracing::debug!(
                    device_id = %req.device_id,
                    param = %req.parameter_name,
                    %old_value,
                    %actual_value,
                    "set_parameter: Parameterized path succeeded"
                );

                // Broadcast parameter change notification
                let _ = self.param_change_tx.send(ParameterChange {
                    device_id: req.device_id.clone(),
                    name: req.parameter_name.clone(),
                    old_value,
                    new_value: actual_value.clone(),
                    units,
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
        tracing::debug!(device_id = %req.device_id, "set_parameter: device not found (no Settable or Parameterized)");
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
        let sample_interval = std::time::Duration::from_secs_f64(1.0 / f64::from(sample_rate_hz));

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
                        #[allow(clippy::cast_possible_truncation)]
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

    // =========================================================================
    // System State Streaming (Phase 4 — Game Loop)
    //
    // Broadcasts aggregated device state snapshots at the game loop tick rate.
    // Drivers write NodeStateUpdate → mpsc → game loop → broadcast.
    // Each gRPC client subscribes to the broadcast channel here.
    // =========================================================================

    type StreamSystemStateStream =
        tokio_stream::wrappers::ReceiverStream<Result<ProtoSystemState, Status>>;

    async fn stream_system_state(
        &self,
        _request: Request<StreamSystemStateRequest>,
    ) -> Result<Response<Self::StreamSystemStateStream>, Status> {
        let broadcast_tx = self.state_broadcast_tx.as_ref().ok_or_else(|| {
            Status::unavailable("System state streaming not configured (game loop not started)")
        })?;

        let mut rx = broadcast_tx.subscribe();
        let (tx, stream_rx) = tokio::sync::mpsc::channel(32);

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(snapshot) => {
                        let proto = snapshot_to_proto(snapshot);
                        if tx.send(Ok(proto)).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("StreamSystemState: lagged, dropped {} snapshots", n);
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
    // Offline Device Feature Query (bd-mmjc)
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "get_device_features"))]
    async fn get_device_features(
        &self,
        request: Request<GetDeviceFeaturesRequest>,
    ) -> Result<Response<GetDeviceFeaturesResponse>, Status> {
        let req = request.into_inner();

        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id must not be empty"));
        }

        // 1. If device is online and implements Parameterized, return live features.
        if let Some(parameterized) = self.registry.get_parameterized(&req.device_id) {
            let params = parameterized.parameters();
            let features: Vec<DeviceFeature> = params
                .iter()
                .map(|(name, param)| {
                    let meta = param.metadata();

                    // Infer dtype from current value when metadata dtype is empty,
                    // matching the list_parameters handler logic.
                    let feature_type = if !meta.dtype.is_empty() {
                        meta.dtype.clone()
                    } else {
                        match param.get_json() {
                            Ok(serde_json::Value::Bool(_)) => "bool".to_string(),
                            Ok(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64() => {
                                "int".to_string()
                            }
                            Ok(serde_json::Value::Number(_)) => "float".to_string(),
                            Ok(serde_json::Value::String(_)) => "string".to_string(),
                            _ => "unknown".to_string(),
                        }
                    };

                    DeviceFeature {
                        device_id: req.device_id.clone(),
                        feature_name: name.to_owned(),
                        feature_type,
                        readable: true,
                        writable: !meta.read_only,
                        min_value: meta.min_value,
                        max_value: meta.max_value,
                        step: meta.step,
                        enum_values: meta.enum_values.clone(),
                        unit: meta.units.clone(),
                        description: meta.description.clone(),
                        group_name: meta.group_name.clone(),
                    }
                })
                .collect();

            return Ok(Response::new(GetDeviceFeaturesResponse {
                features,
                is_live: true,
            }));
        }

        // Device is online but not Parameterized — return empty live result.
        if self.registry.contains(&req.device_id) {
            return Ok(Response::new(GetDeviceFeaturesResponse {
                features: vec![],
                is_live: true,
            }));
        }

        // 2. Device is not in the registry — try offline DB fallback.
        if !req.include_offline {
            return Err(Status::not_found(format!(
                "Device '{}' not found in registry",
                req.device_id
            )));
        }

        // Attempt DB query (feature-gated).
        #[cfg(feature = "db-surreal")]
        {
            if let Some(ref db) = self.db {
                let db_features = db.get_device_features(&req.device_id).await.map_err(|e| {
                    Status::internal(format!("Failed to query device features from DB: {e}"))
                })?;

                if db_features.is_empty() {
                    return Err(Status::not_found(format!(
                        "Device '{}' not found in registry or feature cache",
                        req.device_id
                    )));
                }

                let features: Vec<DeviceFeature> = db_features
                    .into_iter()
                    .map(|f| DeviceFeature {
                        device_id: f.device_id,
                        feature_name: f.feature_name,
                        feature_type: f.feature_type,
                        readable: f.readable,
                        writable: f.writable,
                        min_value: f.min_value,
                        max_value: f.max_value,
                        step: f.step,
                        enum_values: f.enum_values,
                        unit: f.unit,
                        description: f.description,
                        group_name: f.group_name,
                    })
                    .collect();

                return Ok(Response::new(GetDeviceFeaturesResponse {
                    features,
                    is_live: false,
                }));
            }
        }

        // No DB configured or feature not enabled — cannot serve offline data.
        Err(Status::not_found(format!(
            "Device '{}' not found in registry and offline feature cache is not available",
            req.device_id
        )))
    }
}

/// Convert a game-loop snapshot to the proto SystemState message.
fn snapshot_to_proto(snapshot: SystemStateSnapshot) -> ProtoSystemState {
    let nodes = snapshot
        .nodes
        .into_iter()
        .map(|node| {
            let value = match node.value {
                NodeValue::Analog(v) => Some(ProtoNodeValue::AnalogValue(v)),
                NodeValue::Digital(v) => Some(ProtoNodeValue::DigitalValue(v)),
                NodeValue::Text(v) => Some(ProtoNodeValue::TextValue(v)),
                NodeValue::Vector(v) => {
                    Some(ProtoNodeValue::VectorValue(VectorValue { values: v }))
                }
            };
            ProtoNodeState {
                device_id: node.device_id,
                timestamp_ns: node.timestamp_ns,
                value,
                metadata: node.metadata,
            }
        })
        .collect();

    ProtoSystemState {
        nodes,
        broadcast_timestamp_ns: snapshot.broadcast_timestamp_ns,
        tick_rate_hz: snapshot.tick_rate_hz,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardware::registry::create_mock_registry;

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
        // Use new capabilities list (preferred over deprecated boolean flags)
        assert!(
            devices[0].capabilities.contains(&"movable".to_string()),
            "Device should have 'movable' in capabilities list"
        );
        // Deprecated: is_movable boolean flag - kept for backwards compatibility
        #[allow(deprecated)]
        let _ = devices[0].is_movable; // Accessing triggers deprecation warning at compile time
    }

    /// Test that DeviceInfo includes the dynamic capabilities list (bd-4myc).
    ///
    /// The `capabilities` field is the canonical source of truth for device capabilities.
    /// Boolean flags like `is_movable` are deprecated and should not be used in new code.
    #[tokio::test]
    async fn test_device_info_capabilities_list() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ListDevicesRequest {
            capability_filter: None,
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        // Find mock_stage and verify its capabilities list
        let stage = devices.iter().find(|d| d.id == "mock_stage").unwrap();
        assert!(
            stage.capabilities.contains(&"movable".to_string()),
            "mock_stage should have 'movable' capability"
        );
        assert!(
            stage.capabilities.contains(&"parameterized".to_string()),
            "mock_stage should have 'parameterized' capability"
        );

        // Find mock_camera and verify its capabilities
        let camera = devices.iter().find(|d| d.id == "mock_camera").unwrap();
        assert!(
            camera.capabilities.contains(&"frame_producer".to_string()),
            "mock_camera should have 'frame_producer' capability"
        );
        assert!(
            camera.capabilities.contains(&"triggerable".to_string()),
            "mock_camera should have 'triggerable' capability"
        );
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
        // MockPowerMeter base=1e-6W, shot noise = 0.01*sqrt(1e-6) = 1e-5
        // Use fixed tolerance of 1.5e-5 (value can be slightly negative due to noise)
        assert!(
            (resp.value - 1e-6).abs() < 1.5e-5,
            "Reading {} deviates more than 1.5e-5 from base 1e-6",
            resp.value
        );
    }

    /// Test that ReadValueResponse includes the measurement units from device metadata.
    ///
    /// This is critical for the GUI to correctly normalize power readings.
    /// The Newport 1830-C returns Watts, which the GUI must convert to milliwatts
    /// for display. Without the units field, readings appear ~1000× too small.
    #[tokio::test]
    async fn test_read_value_includes_units() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ReadValueRequest {
            device_id: "mock_power_meter".to_string(),
        });
        let response = service.read_value(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        // MockPowerMeter is registered with measurement_units: "W"
        assert_eq!(
            resp.units, "W",
            "ReadValueResponse must include measurement units from device metadata"
        );
    }

    /// Test that ReadValueResponse includes a timestamp.
    #[tokio::test]
    async fn test_read_value_includes_timestamp() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let request = Request::new(ReadValueRequest {
            device_id: "mock_power_meter".to_string(),
        });
        let response = service.read_value(request).await.unwrap();
        let resp = response.into_inner();

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        assert!(resp.success);
        assert!(
            resp.timestamp_ns >= before && resp.timestamp_ns <= after,
            "timestamp_ns should be within the request timeframe"
        );
    }

    /// Test read_value with a non-readable device returns an error.
    #[tokio::test]
    async fn test_read_value_wrong_capability() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // mock_stage is Movable, not Readable
        let request = Request::new(ReadValueRequest {
            device_id: "mock_stage".to_string(),
        });
        let result = service.read_value(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
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
        assert_eq!(p.units, "mm");
        assert!(p.metadata.is_some());
    }

    #[tokio::test]
    async fn test_parameterized_uses_live_metadata_not_registry_snapshot() {
        let registry = create_mock_registry().await.unwrap();

        // Mutate live parameter metadata after registration. Registry metadata is a snapshot
        // and should remain stale; hardware_service must read live metadata from Parameterized.
        let parameterized = registry
            .get_parameterized("mock_stage")
            .expect("mock_stage should be parameterized");
        let position = parameterized
            .parameters()
            .get_typed::<Parameter<f64>>("position")
            .expect("mock_stage.position should exist");
        position.with_metadata(|m| m.units = Some("cm".to_string()));

        let cached_units = registry
            .get_parameter_metadata("mock_stage", "position")
            .and_then(|m| m.units)
            .unwrap_or_default();
        assert_eq!(cached_units, "mm", "registry snapshot should remain stale");

        let service = HardwareServiceImpl::new(Arc::new(registry));

        // list_parameters should return live units
        let list_request = Request::new(ListParametersRequest {
            device_id: "mock_stage".to_string(),
        });
        let list_response = service.list_parameters(list_request).await.unwrap();
        let listed = list_response.into_inner().parameters;
        let listed_position = listed
            .iter()
            .find(|p| p.name == "position")
            .expect("position parameter should be listed");
        assert_eq!(listed_position.units, "cm");

        // get_parameter should also return live units
        let get_request = Request::new(GetParameterRequest {
            device_id: "mock_stage".to_string(),
            parameter_name: "position".to_string(),
        });
        let get_response = service.get_parameter(get_request).await.unwrap();
        assert_eq!(get_response.into_inner().units, "cm");
    }

    #[tokio::test]
    async fn test_set_parameter_out_of_bounds_returns_invalid_argument() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(SetParameterRequest {
            device_id: "mock_power_meter".to_string(),
            parameter_name: "base_power".to_string(),
            value: "20.0".to_string(),
        });

        let result = service.set_parameter(request).await;
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(status.message().contains("exceeds max"));
    }

    // =========================================================================
    // StreamLimiter Tests (bd-64hu)
    // =========================================================================

    // =========================================================================
    // GetDeviceFeatures Tests (bd-mmjc)
    // =========================================================================

    #[tokio::test]
    async fn test_get_device_features_online_parameterized() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // mock_power_meter implements Parameterized — should return live features.
        let request = Request::new(GetDeviceFeaturesRequest {
            device_id: "mock_power_meter".to_string(),
            include_offline: false,
        });
        let response = service.get_device_features(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.is_live, "online device should return is_live=true");
        assert!(
            !resp.features.is_empty(),
            "parameterized device should return features"
        );

        // All features should reference the correct device_id.
        for feature in &resp.features {
            assert_eq!(feature.device_id, "mock_power_meter");
            assert!(!feature.feature_name.is_empty());
            assert!(!feature.feature_type.is_empty());
        }
    }

    #[tokio::test]
    async fn test_get_device_features_device_not_found_no_offline() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Non-existent device with include_offline=false should return NOT_FOUND.
        let request = Request::new(GetDeviceFeaturesRequest {
            device_id: "nonexistent_device".to_string(),
            include_offline: false,
        });
        let result = service.get_device_features(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_get_device_features_empty_device_id() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(GetDeviceFeaturesRequest {
            device_id: String::new(),
            include_offline: false,
        });
        let result = service.get_device_features(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_get_device_features_device_not_found_include_offline_no_db() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Non-existent device with include_offline=true but no DB should return NOT_FOUND.
        let request = Request::new(GetDeviceFeaturesRequest {
            device_id: "nonexistent_device".to_string(),
            include_offline: true,
        });
        let result = service.get_device_features(request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_get_device_features_online_all_devices() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // All mock devices should be parameterized and return live features.
        for device_id in &["mock_stage", "mock_power_meter", "mock_camera"] {
            let request = Request::new(GetDeviceFeaturesRequest {
                device_id: device_id.to_string(),
                include_offline: false,
            });
            let response = service.get_device_features(request).await.unwrap();
            let resp = response.into_inner();

            assert!(resp.is_live, "{device_id} should return is_live=true");
            assert!(
                !resp.features.is_empty(),
                "{device_id} should have features"
            );
        }
    }

    // =========================================================================
    // StreamLimiter Tests (bd-64hu)
    // =========================================================================

    #[test]
    fn test_stream_limiter_acquire_release() {
        use super::StreamLimiter;
        use std::net::{IpAddr, Ipv4Addr};

        let limiter = StreamLimiter::new();
        let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

        // Should be able to acquire up to MAX_STREAMS_PER_CLIENT streams
        for i in 0..common::limits::MAX_STREAMS_PER_CLIENT {
            assert!(
                limiter.try_acquire(client_ip).is_ok(),
                "Failed to acquire stream slot {}",
                i
            );
        }

        // Next acquire should fail with ResourceExhausted
        let result = limiter.try_acquire(client_ip);
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);

        // Release one slot
        limiter.release(client_ip);

        // Now should be able to acquire again
        assert!(limiter.try_acquire(client_ip).is_ok());
    }

    #[test]
    fn test_stream_limiter_different_clients() {
        use super::StreamLimiter;
        use std::net::{IpAddr, Ipv4Addr};

        let limiter = StreamLimiter::new();
        let client1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
        let client2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101));

        // Fill up client1's slots
        for _ in 0..common::limits::MAX_STREAMS_PER_CLIENT {
            assert!(limiter.try_acquire(client1).is_ok());
        }

        // Client2 should still be able to acquire
        assert!(limiter.try_acquire(client2).is_ok());

        // Client1 should be blocked
        assert!(limiter.try_acquire(client1).is_err());
    }

    #[test]
    fn test_stream_limiter_cleanup_on_release() {
        use super::StreamLimiter;
        use std::net::{IpAddr, Ipv4Addr};

        let limiter = StreamLimiter::new();
        let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

        // Acquire and release should clean up the entry
        limiter.try_acquire(client_ip).unwrap();
        limiter.release(client_ip);

        // Internal state should be empty (client removed when count hits 0)
        assert!(!limiter.has_streams(client_ip));
    }

    /// Verify that a FrameProducer reporting `has_acquisition_error() == true`
    /// causes `report_device_failure()` to be recorded in the registry.
    ///
    /// This mirrors the logic in the `stream_frames` forwarding task: when the
    /// observer channel closes and the producer signals an acquisition error,
    /// the device's consecutive-failure counter is incremented so the supervisor
    /// can schedule a reconnection attempt with exponential backoff.
    #[tokio::test]
    async fn test_acquisition_error_reports_device_failure() {
        use async_trait::async_trait;
        use common::capabilities::{FrameObserver, FrameProducer, ObserverHandle};
        use common::health::DeviceHealth;
        use std::sync::Arc;

        /// Minimal FrameProducer that always reports an acquisition error.
        struct ErrorProducer;

        #[async_trait]
        impl FrameProducer for ErrorProducer {
            async fn start_stream(&self) -> anyhow::Result<()> {
                Ok(())
            }
            async fn stop_stream(&self) -> anyhow::Result<()> {
                Ok(())
            }
            fn resolution(&self) -> (u32, u32) {
                (640, 480)
            }
            async fn register_observer(
                &self,
                _observer: Box<dyn FrameObserver>,
            ) -> anyhow::Result<ObserverHandle> {
                Ok(ObserverHandle(1))
            }
            async fn unregister_observer(&self, _handle: ObserverHandle) -> anyhow::Result<()> {
                Ok(())
            }
            fn supports_observers(&self) -> bool {
                true
            }
            /// Signals a hardware acquisition error (e.g. USB disconnect).
            fn has_acquisition_error(&self) -> bool {
                true
            }
        }

        // Use the mock registry which includes "mock_camera" with a health entry.
        let registry = Arc::new(create_mock_registry().await.unwrap());

        let producer: Arc<dyn FrameProducer> = Arc::new(ErrorProducer);

        // Replicate the observer-channel-close path from stream_frames:
        // if has_acquisition_error() → report failure.
        if producer.has_acquisition_error() {
            registry.report_device_failure(
                "mock_camera",
                "acquisition loop exited with hardware error",
            );
        }

        let health = registry
            .get_device_health("mock_camera")
            .expect("health entry missing");
        assert_eq!(
            health.consecutive_failures, 1,
            "single error should increment consecutive_failures"
        );
        assert_eq!(
            health.health,
            DeviceHealth::Degraded,
            "one failure below fault_threshold should degrade, not fault"
        );

        // Conversely: a producer that has NO error must NOT report failure.
        struct OkProducer;

        #[async_trait]
        impl FrameProducer for OkProducer {
            async fn start_stream(&self) -> anyhow::Result<()> {
                Ok(())
            }
            async fn stop_stream(&self) -> anyhow::Result<()> {
                Ok(())
            }
            fn resolution(&self) -> (u32, u32) {
                (640, 480)
            }
            async fn register_observer(
                &self,
                _observer: Box<dyn FrameObserver>,
            ) -> anyhow::Result<ObserverHandle> {
                Ok(ObserverHandle(1))
            }
            async fn unregister_observer(&self, _handle: ObserverHandle) -> anyhow::Result<()> {
                Ok(())
            }
            fn supports_observers(&self) -> bool {
                true
            }
            // Default has_acquisition_error() returns false — no-op path.
        }

        let ok_producer: Arc<dyn FrameProducer> = Arc::new(OkProducer);
        if ok_producer.has_acquisition_error() {
            registry.report_device_failure("mock_camera", "should not be called");
        }

        // Failure count must still be 1 (unchanged by the ok-producer path).
        let health_after = registry
            .get_device_health("mock_camera")
            .expect("health entry missing");
        assert_eq!(
            health_after.consecutive_failures, 1,
            "ok-producer close must NOT increment failure counter"
        );
    }
}
