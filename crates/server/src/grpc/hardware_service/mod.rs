//! HardwareService implementation for direct device control (bd-4x6q)
//!
//! This module provides gRPC endpoints for direct hardware manipulation,
//! bypassing the scripting layer. It connects to the DeviceRegistry for
//! capability-based access to hardware devices.

use crate::grpc::proto::node_state::Value as ProtoNodeValue;
use crate::grpc::{
    anyhow_to_status,
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
        GetParameterFavoritesRequest,
        GetParameterFavoritesResponse,
        GetParameterRequest,
        GetShutterRequest,
        GetShutterResponse,
        GetWavelengthRequest,
        GetWavelengthResponse,
        ListDevicesRequest,
        ListDevicesResponse,
        ListParametersRequest,
        ListParametersResponse,
        LoadCalibrationProfileRequest,
        LoadCalibrationProfileResponse,
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
        SaveCalibrationProfileRequest,
        SaveCalibrationProfileResponse,
        SetEmissionRequest,
        SetEmissionResponse,
        SetExposureRequest,
        SetExposureResponse,
        SetParameterFavoriteRequest,
        SetParameterFavoriteResponse,
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

mod control;
mod discovery;
mod helpers;
mod motion;
mod parameters;
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
    registry: DeviceRegistry,
    /// Per-client stream limiter for DoS prevention (bd-64hu)
    stream_limiter: Arc<StreamLimiter>,
    /// Broadcast sender for parameter changes (enables real-time GUI synchronization)
    param_change_tx: tokio::sync::broadcast::Sender<ParameterChange>,
    /// Broadcast sender for system state snapshots (game loop output)
    state_broadcast_tx: Option<tokio::sync::broadcast::Sender<SystemStateSnapshot>>,
    /// Optional DB access for offline feature queries (bd-mmjc)
    #[cfg(feature = "db")]
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
            Ok(Err(err)) => Err(anyhow_to_status(err)),
            Err(_) => Err(Status::deadline_exceeded(format!(
                "{operation} timed out after {RPC_TIMEOUT:?}"
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
    pub fn new(registry: DeviceRegistry) -> Self {
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
                        let dev_id = device_id.to_string();
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
            #[cfg(feature = "db")]
            db: None,
        }
    }

    /// Create a new HardwareService with an existing parameter change broadcast sender
    /// (useful when sharing the sender across multiple services)
    pub fn with_param_broadcast(
        registry: DeviceRegistry,
        param_change_tx: tokio::sync::broadcast::Sender<ParameterChange>,
    ) -> Self {
        Self {
            registry,
            stream_limiter: Arc::new(StreamLimiter::new()),
            param_change_tx,
            state_broadcast_tx: None,
            #[cfg(feature = "db")]
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

    /// Attach a database instance for offline device feature queries (bd-mmjc).
    ///
    /// Also spawns a debounced parameter state writer (bd-4wf7) that persists
    /// writable parameter changes to the `device_runtime_state` table every 2 seconds.
    #[cfg(feature = "db")]
    pub fn with_db(mut self, db: Option<db::DaqDb>) -> Self {
        if let Some(ref db_instance) = db {
            self.spawn_parameter_state_writer(Arc::new(db_instance.clone()));
        }
        self.db = db.map(Arc::new);
        self
    }

    /// Spawn a background task that debounce-writes parameter changes to the database (bd-4wf7).
    ///
    /// Subscribes to the `param_change_tx` broadcast channel and collects dirty parameters
    /// into a batch. Every 2 seconds, the batch is flushed to `device_runtime_state` via
    /// `batch_upsert_device_state()`. Only writable parameters from user/script sources
    /// are persisted — read-only hardware telemetry is excluded.
    #[cfg(feature = "db")]
    fn spawn_parameter_state_writer(&self, db: Arc<db::DaqDb>) {
        use std::collections::HashMap;
        use tokio::sync::broadcast::error::RecvError;

        let mut rx = self.param_change_tx.subscribe();

        tokio::spawn(async move {
            let mut dirty: HashMap<(String, String), serde_json::Value> = HashMap::new();
            let flush_interval = tokio::time::Duration::from_secs(2);
            let mut flush_timer = tokio::time::interval(flush_interval);
            flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    result = rx.recv() => {
                        match result {
                            Ok(change) => {
                                // Only persist user/script-initiated changes to writable params
                                if change.source == "hardware" {
                                    continue;
                                }
                                // Parse value as JSON for storage
                                let value = serde_json::from_str(&change.new_value)
                                    .unwrap_or_else(|_| serde_json::Value::String(change.new_value.clone()));
                                dirty.insert(
                                    (change.device_id, change.name),
                                    value,
                                );
                            }
                            Err(RecvError::Lagged(n)) => {
                                tracing::debug!("Parameter state writer lagged, dropped {n} messages");
                            }
                            Err(RecvError::Closed) => break,
                        }
                    }
                    _ = flush_timer.tick() => {
                        if dirty.is_empty() {
                            continue;
                        }
                        let batch: Vec<(String, String, serde_json::Value)> = dirty
                            .drain()
                            .map(|((dev, name), val)| (dev, name, val))
                            .collect();
                        let count = batch.len();
                        if let Err(e) = db.batch_upsert_device_state(&batch).await {
                            tracing::warn!("Failed to persist {count} parameter states: {e}");
                        } else {
                            tracing::debug!("Persisted {count} parameter state(s) to DB");
                        }
                    }
                }
            }
        });
    }

    /// Get a clone of the parameter change broadcast sender for external notification
    pub fn param_change_sender(&self) -> tokio::sync::broadcast::Sender<ParameterChange> {
        self.param_change_tx.clone()
    }
}

/// Helper macro to reduce boilerplate for capability lookups
///
/// Usage: require_capability!(svc, get_movable, &device_id, "not movable")
///
/// Expands to:
/// ```ignore
/// let capability = svc.registry.$getter($device_id);
/// let capability = capability.ok_or_else(|| {
///     Status::not_found(format!("Device '{}' {}",
///         $device_id, $capability_desc))
/// })?;
/// ```
macro_rules! require_capability {
    ($svc:expr, $getter:ident, $device_id:expr, $capability_desc:literal) => {{
        let capability = $svc.registry.$getter($device_id);
        capability.ok_or_else(|| {
            Status::not_found(format!("Device '{}' {}", $device_id, $capability_desc))
        })?
    }};
}
// Make macro visible to sub-modules
use require_capability;

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
        discovery::list_devices(self, request)
    }

    #[instrument(skip(self, request), fields(method = "get_device_state"))]
    async fn get_device_state(
        &self,
        request: Request<DeviceStateRequest>,
    ) -> Result<Response<DeviceStateResponse>, Status> {
        discovery::get_device_state(self, request).await
    }

    async fn subscribe_device_state(
        &self,
        request: Request<DeviceStateSubscribeRequest>,
    ) -> Result<Response<Self::SubscribeDeviceStateStream>, Status> {
        discovery::subscribe_device_state(self, request)
    }

    // =========================================================================
    // Motion Control
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "move_absolute"))]
    async fn move_absolute(
        &self,
        request: Request<MoveRequest>,
    ) -> Result<Response<MoveResponse>, Status> {
        motion::move_absolute(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "move_relative"))]
    async fn move_relative(
        &self,
        request: Request<MoveRequest>,
    ) -> Result<Response<MoveResponse>, Status> {
        motion::move_relative(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "stop_motion"))]
    async fn stop_motion(
        &self,
        request: Request<StopMotionRequest>,
    ) -> Result<Response<StopMotionResponse>, Status> {
        motion::stop_motion(self, request).await
    }

    async fn wait_settled(
        &self,
        request: Request<WaitSettledRequest>,
    ) -> Result<Response<WaitSettledResponse>, Status> {
        motion::wait_settled(self, request).await
    }

    type StreamPositionStream =
        tokio_stream::wrappers::ReceiverStream<Result<PositionUpdate, Status>>;

    async fn stream_position(
        &self,
        request: Request<StreamPositionRequest>,
    ) -> Result<Response<Self::StreamPositionStream>, Status> {
        motion::stream_position(self, request)
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
        motion::read_value(self, request).await
    }

    type StreamValuesStream = tokio_stream::wrappers::ReceiverStream<Result<ValueUpdate, Status>>;

    async fn stream_values(
        &self,
        request: Request<StreamValuesRequest>,
    ) -> Result<Response<Self::StreamValuesStream>, Status> {
        motion::stream_values(self, request)
    }

    // =========================================================================
    // Trigger Control
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "arm"))]
    async fn arm(&self, request: Request<ArmRequest>) -> Result<Response<ArmResponse>, Status> {
        control::arm(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "trigger"))]
    async fn trigger(
        &self,
        request: Request<TriggerRequest>,
    ) -> Result<Response<TriggerResponse>, Status> {
        control::trigger(self, request).await
    }

    // =========================================================================
    // Exposure Control
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "set_exposure"))]
    async fn set_exposure(
        &self,
        request: Request<SetExposureRequest>,
    ) -> Result<Response<SetExposureResponse>, Status> {
        control::set_exposure(self, request).await
    }

    async fn get_exposure(
        &self,
        request: Request<GetExposureRequest>,
    ) -> Result<Response<GetExposureResponse>, Status> {
        control::get_exposure(self, request).await
    }

    // =========================================================================
    // Laser Control (bd-pwjo)
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "set_shutter"))]
    async fn set_shutter(
        &self,
        request: Request<SetShutterRequest>,
    ) -> Result<Response<SetShutterResponse>, Status> {
        control::set_shutter(self, request).await
    }

    async fn get_shutter(
        &self,
        request: Request<GetShutterRequest>,
    ) -> Result<Response<GetShutterResponse>, Status> {
        control::get_shutter(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "set_wavelength"))]
    async fn set_wavelength(
        &self,
        request: Request<SetWavelengthRequest>,
    ) -> Result<Response<SetWavelengthResponse>, Status> {
        control::set_wavelength(self, request).await
    }

    async fn get_wavelength(
        &self,
        request: Request<GetWavelengthRequest>,
    ) -> Result<Response<GetWavelengthResponse>, Status> {
        control::get_wavelength(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "set_emission"))]
    async fn set_emission(
        &self,
        request: Request<SetEmissionRequest>,
    ) -> Result<Response<SetEmissionResponse>, Status> {
        control::set_emission(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "get_emission"))]
    async fn get_emission(
        &self,
        request: Request<GetEmissionRequest>,
    ) -> Result<Response<GetEmissionResponse>, Status> {
        control::get_emission(self, request).await
    }

    // =========================================================================
    // Frame Streaming
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "start_stream"))]
    async fn start_stream(
        &self,
        request: Request<StartStreamRequest>,
    ) -> Result<Response<StartStreamResponse>, Status> {
        control::start_stream(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "stop_stream"))]
    async fn stop_stream(
        &self,
        request: Request<StopStreamRequest>,
    ) -> Result<Response<StopStreamResponse>, Status> {
        control::stop_stream(self, request).await
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
        control::stream_frames(self, request).await
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
        control::stage_device(self, request).await
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
        control::unstage_device(self, request).await
    }

    // =========================================================================
    // Passthrough Commands (escape hatch for device-specific features)
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "execute_device_command"))]
    async fn execute_device_command(
        &self,
        request: Request<DeviceCommandRequest>,
    ) -> Result<Response<DeviceCommandResponse>, Status> {
        control::execute_device_command(self, request).await
    }

    // =========================================================================
    // Observable Parameters (QCodes/ScopeFoundry pattern)
    // =========================================================================

    async fn list_parameters(
        &self,
        request: Request<ListParametersRequest>,
    ) -> Result<Response<ListParametersResponse>, Status> {
        parameters::list_parameters(self, request)
    }

    async fn get_parameter(
        &self,
        request: Request<GetParameterRequest>,
    ) -> Result<Response<ParameterValue>, Status> {
        parameters::get_parameter(self, request).await
    }

    #[instrument(skip(self, request), fields(method = "set_parameter"))]
    async fn set_parameter(
        &self,
        request: Request<SetParameterRequest>,
    ) -> Result<Response<SetParameterResponse>, Status> {
        parameters::set_parameter(self, request).await
    }

    type StreamParameterChangesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ParameterChange, Status>>;

    async fn stream_parameter_changes(
        &self,
        request: Request<StreamParameterChangesRequest>,
    ) -> Result<Response<Self::StreamParameterChangesStream>, Status> {
        parameters::stream_parameter_changes(self, request)
    }

    async fn set_parameter_favorite(
        &self,
        request: Request<SetParameterFavoriteRequest>,
    ) -> Result<Response<SetParameterFavoriteResponse>, Status> {
        parameters::set_parameter_favorite(self, request).await
    }

    async fn get_parameter_favorites(
        &self,
        request: Request<GetParameterFavoritesRequest>,
    ) -> Result<Response<GetParameterFavoritesResponse>, Status> {
        parameters::get_parameter_favorites(self, request).await
    }

    async fn load_calibration_profile(
        &self,
        request: Request<LoadCalibrationProfileRequest>,
    ) -> Result<Response<LoadCalibrationProfileResponse>, Status> {
        let path = request.into_inner().path;
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(Response::new(LoadCalibrationProfileResponse {
                success: true,
                content,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(LoadCalibrationProfileResponse {
                success: false,
                content: String::new(),
                error_message: format!("Failed to read {path}: {e}"),
            })),
        }
    }

    async fn save_calibration_profile(
        &self,
        request: Request<SaveCalibrationProfileRequest>,
    ) -> Result<Response<SaveCalibrationProfileResponse>, Status> {
        let req = request.into_inner();
        match std::fs::write(&req.path, &req.content) {
            Ok(()) => Ok(Response::new(SaveCalibrationProfileResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(SaveCalibrationProfileResponse {
                success: false,
                error_message: format!("Failed to write {}: {}", req.path, e),
            })),
        }
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
        parameters::stream_observables(self, request)
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
        request: Request<StreamSystemStateRequest>,
    ) -> Result<Response<Self::StreamSystemStateStream>, Status> {
        parameters::stream_system_state(self, request)
    }

    // =========================================================================
    // Offline Device Feature Query (bd-mmjc)
    // =========================================================================

    #[instrument(skip(self, request), fields(method = "get_device_features"))]
    async fn get_device_features(
        &self,
        request: Request<GetDeviceFeaturesRequest>,
    ) -> Result<Response<GetDeviceFeaturesResponse>, Status> {
        discovery::get_device_features(self, request).await
    }
}

#[cfg(test)]
// Uses deprecated `create_mock_registry` because these tests depend on mock driver
// internals (Parameter<f64> typed access, specific parameter names/bounds/units)
// that differ from the universal-driver-backed canonical mock registry.
// New server tests should use `driver_registry::create_canonical_mock_registry`.
#[allow(deprecated)]
mod tests {
    use super::*;
    use hardware::registry::create_mock_registry;

    #[tokio::test]
    async fn test_list_devices() {
        let registry = create_mock_registry().await.unwrap();
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);
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
        let service = HardwareServiceImpl::new(registry);
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
        let service = HardwareServiceImpl::new(registry);

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

        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
                "Failed to acquire stream slot {i}"
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
