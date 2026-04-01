//! Observable parameters and state streaming endpoints.
// tonic gRPC handlers must return Result<Response<T>, Status>; Status is inherently large.
#![allow(clippy::result_large_err)]

use super::*;

pub(super) fn list_parameters(
    svc: &HardwareServiceImpl,
    request: Request<ListParametersRequest>,
) -> Result<Response<ListParametersResponse>, Status> {
    let req = request.into_inner();

    // Check if device exists
    if !svc.registry.contains(&req.device_id) {
        return Err(Status::not_found(format!(
            "Device '{}' not found",
            req.device_id
        )));
    }

    let mut parameters = Vec::new();

    // 1. Get V5 parameters from Parameterized devices
    if let Some(parameterized) = svc.registry.get_parameterized(&req.device_id) {
        let param_set = parameterized.parameters();
        for param_name in param_set.names() {
            if let Some(param) = param_set.get(param_name) {
                // Use live metadata from the parameter itself. Registry metadata is a
                // registration-time snapshot and can be stale for dynamic choices.
                let live_metadata = param.metadata();

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
                    name: live_metadata.name.clone(),
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

pub(super) async fn get_parameter(
    svc: &HardwareServiceImpl,
    request: Request<GetParameterRequest>,
) -> Result<Response<ParameterValue>, Status> {
    let req = request.into_inner();

    // New path - use Parameterized trait first (synchronous cache)
    if let Some(parameterized) = svc.registry.get_parameterized(&req.device_id) {
        let params = parameterized.parameters();
        if let Some(param) = params.get(&req.parameter_name) {
            let value = param.get_json().map_err(|e| {
                map_hardware_error_to_status(&format!("Failed to get parameter: {}", e))
            })?;
            let units = param.metadata().units.unwrap_or_default();
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
    if let Some(settable) = svc.registry.get_settable(&req.device_id) {
        // Get the parameter value
        let value = settable.get_value(&req.parameter_name).await.map_err(|e| {
            let err_msg = format!("Failed to get parameter: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            map_hardware_error_to_status(&err_msg)
        })?;
        svc.registry.report_device_success(&req.device_id);
        let units = svc
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

pub(super) async fn set_parameter(
    svc: &HardwareServiceImpl,
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
    if let Some(settable) = svc.registry.get_settable(&req.device_id) {
        tracing::debug!(device_id = %req.device_id, param = %req.parameter_name, "set_parameter: using Settable path");
        let metadata = svc
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
                svc.registry.report_device_failure(&req.device_id, &err_msg);
                map_hardware_error_to_status(&err_msg)
            })?;
        svc.registry.report_device_success(&req.device_id);

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
        let _ = svc.param_change_tx.send(ParameterChange {
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
    if let Some(parameterized) = svc.registry.get_parameterized(&req.device_id) {
        tracing::debug!(device_id = %req.device_id, param = %req.parameter_name, "set_parameter: using Parameterized path");
        let params = parameterized.parameters();

        if let Some(param) = params.get(&req.parameter_name) {
            let metadata = param.metadata();
            let old_value = param.get_json().map(|v| v.to_string()).unwrap_or_default();

            // Parse the value string to JSON
            let json_value: serde_json::Value = serde_json::from_str(&req.value)
                .or_else(|_| {
                    // Try as raw string if JSON parsing fails
                    Ok::<_, serde_json::Error>(serde_json::Value::String(req.value.clone()))
                })
                .map_err(|e| Status::invalid_argument(format!("Invalid value format: {}", e)))?;

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
            let _ = svc.param_change_tx.send(ParameterChange {
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

pub(super) fn stream_parameter_changes(
    svc: &HardwareServiceImpl,
    request: Request<StreamParameterChangesRequest>,
) -> Result<Response<ReceiverStream<Result<ParameterChange, Status>>>, Status> {
    let req = request.into_inner();

    // Extract filter criteria
    let device_filter = req.device_id.clone();
    let param_filter: std::collections::HashSet<String> = req.parameter_names.into_iter().collect();

    // Subscribe to parameter change broadcast
    let mut rx = svc.param_change_tx.subscribe();

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

pub(super) fn stream_observables(
    svc: &HardwareServiceImpl,
    request: Request<StreamObservablesRequest>,
) -> Result<Response<ReceiverStream<Result<ObservableValue, Status>>>, Status> {
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
    let registry = svc.registry.clone();

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

pub(super) fn stream_system_state(
    svc: &HardwareServiceImpl,
    _request: Request<StreamSystemStateRequest>,
) -> Result<Response<ReceiverStream<Result<ProtoSystemState, Status>>>, Status> {
    let broadcast_tx = svc.state_broadcast_tx.as_ref().ok_or_else(|| {
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

/// Set or clear the favorite flag for a parameter (bd-4wf7).
#[allow(unused_variables)] // svc and req only used with db-surreal feature
#[allow(clippy::unused_async)] // conditionally async: .await used only with db-surreal feature
pub(super) async fn set_parameter_favorite(
    svc: &HardwareServiceImpl,
    request: Request<SetParameterFavoriteRequest>,
) -> Result<Response<SetParameterFavoriteResponse>, Status> {
    let req = request.into_inner();

    #[cfg(feature = "db-surreal")]
    if let Some(ref db) = svc.db {
        db.set_parameter_favorite(&req.device_id, &req.parameter_name, req.is_favorite)
            .await
            .map_err(|e| Status::internal(format!("DB error: {e}")))?;
        return Ok(Response::new(SetParameterFavoriteResponse {
            success: true,
        }));
    }

    // No DB available — favorites are not persisted
    Ok(Response::new(SetParameterFavoriteResponse {
        success: false,
    }))
}

/// Get all favorited parameter names for a device (bd-4wf7).
#[allow(unused_variables)] // svc and req only used with db-surreal feature
#[allow(clippy::unused_async)] // conditionally async: .await used only with db-surreal feature
pub(super) async fn get_parameter_favorites(
    svc: &HardwareServiceImpl,
    request: Request<GetParameterFavoritesRequest>,
) -> Result<Response<GetParameterFavoritesResponse>, Status> {
    let req = request.into_inner();

    #[cfg(feature = "db-surreal")]
    if let Some(ref db) = svc.db {
        let names = db
            .get_favorites(&req.device_id)
            .await
            .map_err(|e| Status::internal(format!("DB error: {e}")))?;
        return Ok(Response::new(GetParameterFavoritesResponse {
            parameter_names: names,
        }));
    }

    Ok(Response::new(GetParameterFavoritesResponse {
        parameter_names: vec![],
    }))
}
