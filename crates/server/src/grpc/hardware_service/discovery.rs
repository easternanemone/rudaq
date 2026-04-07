//! Device discovery and introspection endpoints.
// tonic gRPC handlers must return Result<Response<T>, Status>; Status is inherently large.
#![allow(clippy::result_large_err)]

use super::*;

pub(super) fn list_devices(
    svc: &HardwareServiceImpl,
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
                    "Unknown capability: {capability_filter}"
                )));
            }
        };

        svc.registry
            .devices_with_capability(cap)
            .iter()
            .filter_map(|id| svc.registry.get_device_info(id))
            .map(|info| {
                let health = svc.registry.get_device_health(&info.id);
                helpers::device_info_to_proto_with_health(&info, health.as_ref())
            })
            .collect()
    } else {
        // Return all devices
        svc.registry
            .list_devices()
            .iter()
            .map(|info| {
                let health = svc.registry.get_device_health(&info.id);
                helpers::device_info_to_proto_with_health(info, health.as_ref())
            })
            .collect()
    };

    // Include registration failures for debugging visibility
    let registration_failures: Vec<ProtoRegistrationFailure> = svc
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

pub(super) async fn get_device_state(
    svc: &HardwareServiceImpl,
    request: Request<DeviceStateRequest>,
) -> Result<Response<DeviceStateResponse>, Status> {
    let req = request.into_inner();
    tracing::info!(device_id = %req.device_id, "GetDeviceState called");

    // Acquire device references without lock
    // This prevents deadlock when hardware operations take time
    let (movable, readable, triggerable, frame_producer, exposure_control, exists) = (
        svc.registry.get_movable(&req.device_id),
        svc.registry.get_readable(&req.device_id),
        svc.registry.get_triggerable(&req.device_id),
        svc.registry.get_frame_producer(&req.device_id),
        svc.registry.get_exposure_control(&req.device_id),
        svc.registry.contains(&req.device_id),
    );

    if !exists {
        return Err(Status::not_found(format!(
            "Device not found: {}",
            req.device_id
        )));
    }

    // Populate health fields from registry (bd-vgrj)
    let health = svc.registry.get_device_health(&req.device_id);
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

pub(super) fn subscribe_device_state(
    svc: &HardwareServiceImpl,
    request: Request<DeviceStateSubscribeRequest>,
) -> Result<Response<ReceiverStream<Result<DeviceStateUpdate, Status>>>, Status> {
    let req = request.into_inner();

    // Determine device list and validate device IDs exist
    let device_ids: Vec<String> = if req.device_ids.is_empty() {
        svc.registry
            .list_devices()
            .iter()
            .map(|d| d.id.clone())
            .collect()
    } else {
        // Validate all requested device IDs exist
        for device_id in &req.device_ids {
            if !svc.registry.contains(device_id) {
                return Err(Status::not_found(format!("Device '{device_id}' not found")));
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
    let registry = svc.registry.clone();
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

#[allow(clippy::unused_async)] // async required when db-surreal feature is enabled
pub(super) async fn get_device_features(
    svc: &HardwareServiceImpl,
    request: Request<GetDeviceFeaturesRequest>,
) -> Result<Response<GetDeviceFeaturesResponse>, Status> {
    let req = request.into_inner();

    if req.device_id.is_empty() {
        return Err(Status::invalid_argument("device_id must not be empty"));
    }

    // 1. If device is online and implements Parameterized, return live features.
    if let Some(parameterized) = svc.registry.get_parameterized(&req.device_id) {
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
    if svc.registry.contains(&req.device_id) {
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
    #[cfg(feature = "db")]
    {
        if let Some(ref db) = svc.db {
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
