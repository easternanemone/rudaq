//! Shared helper functions for validation, error mapping, and proto conversion.

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

pub(super) async fn fetch_device_state(
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
        return Err(Status::not_found(format!("Device not found: {device_id}")));
    }

    // Populate health fields from registry (bd-vgrj)
    let health = registry.get_device_health(device_id);
    let (health_status, consecutive_failures, restart_attempts, last_error, is_faulted) =
        if let Some(ref h) = health {
            (
                device_health_to_proto(h.health),
                h.consecutive_failures,
                h.restart_attempts,
                h.last_error.clone().unwrap_or_default(),
                h.health == common::health::DeviceHealth::Faulted,
            )
        } else {
            (
                device_health_to_proto(common::health::DeviceHealth::Healthy),
                0,
                0,
                String::new(),
                false,
            )
        };

    let mut response = DeviceStateResponse {
        device_id: device_id.to_string(),
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
pub(super) fn device_state_to_fields_json(state: &DeviceStateResponse) -> HashMap<String, String> {
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

pub(super) fn now_ns() -> u64 {
    common::time::now_ns()
}

pub(super) fn proto_parameter_metadata(meta: &CommonParameterMetadata) -> ProtoParameterMetadata {
    ProtoParameterMetadata {
        min_value: meta.min_value,
        max_value: meta.max_value,
        step: meta.step,
        units: meta.units.clone().unwrap_or_default(),
        read_only: meta.read_only,
        dtype: meta.dtype.clone(),
        enum_values: meta.enum_values.clone(),
        description: meta.description.clone().unwrap_or_default(),
        group_name: meta.group_name.clone(),
    }
}

fn extract_numeric_value(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(num) => num.as_f64(),
        _ => None,
    }
}

#[allow(clippy::result_large_err)] // tonic::Status (176 bytes) is the standard gRPC error type
pub(super) fn validate_parameter_value(
    name: &str,
    metadata: Option<&CommonParameterMetadata>,
    value: &serde_json::Value,
) -> Result<(), Status> {
    let Some(meta) = metadata else {
        return Ok(());
    };

    if meta.read_only {
        return Err(Status::invalid_argument(format!(
            "Parameter '{name}' is read-only"
        )));
    }

    if !meta.enum_values.is_empty() || meta.dtype == "enum" {
        let value_str = value.as_str().ok_or_else(|| {
            Status::invalid_argument(format!("Parameter '{name}' expects an enum string value"))
        })?;
        if !meta.enum_values.iter().any(|v| v == value_str) {
            return Err(Status::invalid_argument(format!(
                "Parameter '{}' value '{}' is not in allowed set {:?}",
                name, value_str, meta.enum_values
            )));
        }
    }

    // String-type validation (bd-eefxe): reject non-string JSON values
    if meta.dtype == "string" {
        if !value.is_string() {
            return Err(Status::invalid_argument(format!(
                "Parameter '{}' expects a string value, got {}",
                name, value
            )));
        }
        return Ok(());
    }

    if meta.min_value.is_some() || meta.max_value.is_some() {
        let numeric = extract_numeric_value(value).ok_or_else(|| {
            Status::invalid_argument(format!("Parameter '{name}' expects a numeric value"))
        })?;
        if let Some(min) = meta.min_value
            && numeric < min
        {
            return Err(Status::invalid_argument(format!(
                "Parameter '{name}' value {numeric} below minimum {min}"
            )));
        }
        if let Some(max) = meta.max_value
            && numeric > max
        {
            return Err(Status::invalid_argument(format!(
                "Parameter '{name}' value {numeric} exceeds max {max}"
            )));
        }
        if let Some(step) = meta.step {
            // Align relative to min or zero
            let origin = meta.min_value.unwrap_or(0.0);
            let rem = (numeric - origin) % step;
            // Use a small epsilon for floating point comparison
            if rem.abs() > 1e-10 && (rem - step).abs() > 1e-10 {
                return Err(Status::invalid_argument(format!(
                    "Parameter '{name}' value {numeric} is not a multiple of step {step}"
                )));
            }
        }
    }

    Ok(())
}

pub(super) fn monitor_parameter<T: std::fmt::Display + Clone + Send + Sync + 'static>(
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

/// Map anyhow errors to gRPC Status, preferring structured DaqError mapping.
///
/// Scans the full error chain so that `anyhow::Context` wrappers don't hide
/// structured errors (`DaqError`, `DriverError`, `StorageError`).
pub(super) fn map_anyhow_error_to_status(err: AnyError) -> Status {
    for cause in err.chain() {
        if let Some(daq_err) = cause.downcast_ref::<DaqError>() {
            return map_daq_error_to_status(daq_err);
        }
        if let Some(driver_err) = cause.downcast_ref::<DriverError>() {
            return map_daq_error_to_status(&DaqError::Driver(driver_err.clone()));
        }
        if let Some(storage_err) = cause.downcast_ref::<StorageError>() {
            return map_daq_error_to_status(&DaqError::Storage(storage_err.clone()));
        }
    }
    map_hardware_error_to_status(&err.to_string())
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
pub(super) fn map_hardware_error_to_status(error_msg: &str) -> Status {
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

/// Map `common::health::DeviceHealth` to the proto `DeviceHealthLevel` i32 value (bd-vgrj).
pub(super) fn device_health_to_proto(health: common::health::DeviceHealth) -> i32 {
    use crate::grpc::proto::DeviceHealthLevel;
    match health {
        common::health::DeviceHealth::Healthy => DeviceHealthLevel::DeviceHealthHealthy as i32,
        common::health::DeviceHealth::Degraded => DeviceHealthLevel::DeviceHealthDegraded as i32,
        common::health::DeviceHealth::Faulted => DeviceHealthLevel::DeviceHealthFaulted as i32,
        common::health::DeviceHealth::Recovering => {
            DeviceHealthLevel::DeviceHealthRecovering as i32
        }
    }
}

/// Convert internal DeviceInfo to proto DeviceInfo, optionally including health state (bd-vgrj).
pub(super) fn device_info_to_proto_with_health(
    info: &hardware::registry::DeviceInfo,
    health: Option<&common::health::DeviceHealthState>,
) -> DeviceInfo {
    // Use explicit category from metadata if set, otherwise infer from driver/capabilities
    let category = get_device_category(
        info.metadata.category,
        &info.driver_type,
        &info.capabilities,
    );

    let health_status =
        health
            .map(|h| device_health_to_proto(h.health))
            .unwrap_or(device_health_to_proto(
                common::health::DeviceHealth::Healthy,
            ));

    #[allow(deprecated)]
    DeviceInfo {
        id: info.id.clone(),
        name: info.name.clone(),
        driver_type: info.driver_type.clone(),
        category: category as i32,
        // LEGACY: Deprecated boolean capability flags, kept populated for wire
        // compatibility with older UI clients. Remove after v1.0 when all clients
        // use the `capabilities` repeated string field (field 100). See
        // docs/reference/deprecation-plan.md Section 1.1.
        is_movable: info.capabilities.contains(&Capability::Movable),
        is_readable: info.capabilities.contains(&Capability::Readable),
        is_triggerable: info.capabilities.contains(&Capability::Triggerable),
        is_frame_producer: info.capabilities.contains(&Capability::FrameProducer),
        is_exposure_controllable: info.capabilities.contains(&Capability::ExposureControl),
        is_shutter_controllable: info.capabilities.contains(&Capability::ShutterControl),
        is_wavelength_tunable: info.capabilities.contains(&Capability::WavelengthTunable),
        is_emission_controllable: info.capabilities.contains(&Capability::EmissionControl),
        is_parameterized: info.capabilities.contains(&Capability::Parameterized),
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
            config_source: info.metadata.config_source.clone(),
            available_commands: info.metadata.available_commands.clone(),
            ui_schema_json: info.metadata.ui_schema_json.clone(),
            panel_kind: info.metadata.panel_kind.clone(),
        }),
        // Dynamic capability list - canonical source of truth (bd-4myc.3)
        capabilities: info
            .capabilities
            .iter()
            .map(|c| c.as_str().to_string())
            .collect(),
        // Device health status (bd-vgrj)
        health_status,
    }
}

/// Get device category, preferring explicit metadata over inference (bd-le6k)
///
/// Priority:
/// 1. Explicit category from DeviceMetadata (set by driver)
/// 2. String-based inference from driver type
/// 3. Capability-based inference
pub(super) fn get_device_category(
    explicit_category: Option<common::capabilities::DeviceCategory>,
    driver_type: &str,
    capabilities: &[Capability],
) -> protocol::DeviceCategory {
    use common::capabilities::DeviceCategory as CoreCategory;
    use protocol::DeviceCategory as ProtoCategory;

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
