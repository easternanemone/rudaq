//! Motion control and sensor reading endpoints.
// tonic gRPC handlers must return Result<Response<T>, Status>; Status is inherently large.
#![allow(clippy::result_large_err)]

use super::*;

pub(super) async fn move_absolute(
    svc: &HardwareServiceImpl,
    request: Request<MoveRequest>,
) -> Result<Response<MoveResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let movable = require_capability!(svc, get_movable, &req.device_id, "not found or not movable");

    // Parameter bounds validation (bd-izdj.3)
    if let Some(meta) = svc
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

    svc.await_with_health_reporting(&req.device_id, "move_abs", movable.move_abs(req.value))
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
                        Status::unavailable(format!("Move completed but position verification failed: {e}"))
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
            svc.await_with_health_reporting(&req.device_id, "wait_settled", movable.wait_settled())
                .await?;
            let pos = movable.position().await.map_err(|e| {
                tracing::error!(device_id = %req.device_id, error = %e, "Failed to verify position after move");
                Status::unavailable(format!(
                    "Move completed but position verification failed: {e}"
                ))
            })?;
            (pos, Some(true))
        }
    } else {
        let pos = movable.position().await.map_err(|e| {
            tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after move");
            Status::unavailable(format!(
                "Move initiated but position read failed: {e}"
            ))
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

pub(super) async fn move_relative(
    svc: &HardwareServiceImpl,
    request: Request<MoveRequest>,
) -> Result<Response<MoveResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let movable = require_capability!(svc, get_movable, &req.device_id, "not found or not movable");

    // Parameter bounds validation (bd-izdj.3)
    // For relative moves, we need to know current position to validate bounds
    if let Some(info) = svc.registry.get_device_info(&req.device_id) {
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
                            "Relative move to {target} is below minimum {min}"
                        )));
                    }

                    if let Some(max) = info.metadata.max_position
                        && target > max
                    {
                        return Err(Status::invalid_argument(format!(
                            "Relative move to {target} is above maximum {max}"
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
                        "Cannot validate relative move: failed to read current position: {e}"
                    )));
                }
            }
        }
    }

    svc.await_with_health_reporting(&req.device_id, "move_rel", movable.move_rel(req.value))
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
                        Status::unavailable(format!("Move completed but position verification failed: {e}"))
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
            svc.await_with_health_reporting(&req.device_id, "wait_settled", movable.wait_settled())
                .await?;
            let pos = movable.position().await.map_err(|e| {
                tracing::error!(device_id = %req.device_id, error = %e, "Failed to verify position after relative move");
                Status::unavailable(format!(
                    "Move completed but position verification failed: {e}"
                ))
            })?;
            (pos, Some(true))
        }
    } else {
        let pos = movable.position().await.map_err(|e| {
            tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after relative move");
            Status::unavailable(format!(
                "Move initiated but position read failed: {e}"
            ))
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

pub(super) async fn stop_motion(
    svc: &HardwareServiceImpl,
    request: Request<StopMotionRequest>,
) -> Result<Response<StopMotionResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let movable = require_capability!(svc, get_movable, &req.device_id, "not found or not movable");

    svc.await_with_health_reporting(&req.device_id, "stop_motion", movable.stop())
        .await?;

    let position = movable.position().await.map_err(|e| {
        tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after stop");
        Status::unavailable(format!(
            "Stop completed but position read failed: {e}"
        ))
    })?;
    Ok(Response::new(StopMotionResponse {
        success: true,
        stopped_position: position,
    }))
}

pub(super) async fn wait_settled(
    svc: &HardwareServiceImpl,
    request: Request<WaitSettledRequest>,
) -> Result<Response<WaitSettledResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let movable = require_capability!(svc, get_movable, &req.device_id, "not found or not movable");

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
        svc.await_with_health_reporting(&req.device_id, "wait_settled", movable.wait_settled())
            .await?;
    }

    let position = movable.position().await.map_err(|e| {
        tracing::error!(device_id = %req.device_id, error = %e, "Failed to read position after wait_settled");
        Status::unavailable(format!(
            "Wait settled completed but position read failed: {e}"
        ))
    })?;
    Ok(Response::new(WaitSettledResponse {
        success: true,
        settled: true,
        position,
    }))
}

pub(super) fn stream_position(
    svc: &HardwareServiceImpl,
    request: Request<StreamPositionRequest>,
) -> Result<Response<ReceiverStream<Result<PositionUpdate, Status>>>, Status> {
    let req = request.into_inner();
    let registry = svc.registry.clone();
    let device_id = req.device_id.clone();
    let rate_hz = req.rate_hz.max(1); // Minimum 1 Hz

    // Verify device exists and is movable
    if svc.registry.get_movable(&device_id).is_none() {
        return Err(Status::not_found(format!(
            "Device '{device_id}' not found or not movable"
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

                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Unix epoch nanos will not exceed u64::MAX until year 2554"
                )]
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

#[expect(
    clippy::cast_possible_truncation,
    reason = "hardware readback values fit in protobuf numeric fields"
)]
pub(super) async fn read_value(
    svc: &HardwareServiceImpl,
    request: Request<ReadValueRequest>,
) -> Result<Response<ReadValueResponse>, Status> {
    let req = request.into_inner();
    tracing::debug!("read_value called for device_id={}", req.device_id);

    // Extract Arc and metadata without lock before awaiting hardware
    let readable = require_capability!(
        svc,
        get_readable,
        &req.device_id,
        "not found or not readable"
    );
    let units = svc
        .registry
        .get_device_info(&req.device_id)
        .and_then(|info| info.metadata.measurement_units.clone())
        .unwrap_or_default();

    let value = svc
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

pub(super) fn stream_values(
    svc: &HardwareServiceImpl,
    request: Request<StreamValuesRequest>,
) -> Result<Response<ReceiverStream<Result<ValueUpdate, Status>>>, Status> {
    let req = request.into_inner();
    let registry = svc.registry.clone();
    let device_id = req.device_id.clone();
    let rate_hz = req.rate_hz.max(1);

    // Verify device exists and is readable
    if svc.registry.get_readable(&device_id).is_none() {
        return Err(Status::not_found(format!(
            "Device '{device_id}' not found or not readable"
        )));
    }

    let (tx, rx) = tokio::sync::mpsc::channel(100);

    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(rate_hz));
        let mut ticker = tokio::time::interval(interval);

        // Mark device as actively streaming (prevents hot-swap reconfiguration).
        registry.set_measurement_lock(&device_id, common::capabilities::MeasurementLock::Measuring);

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
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "Unix epoch nanos will not exceed u64::MAX until year 2554"
                        )]
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
                    Err(e) => {
                        registry.report_device_failure(
                            &device_id,
                            format!("stream_values read failed: {e}"),
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
