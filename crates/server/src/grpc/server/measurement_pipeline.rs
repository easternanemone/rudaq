//! Measurement payload assembly + image data integrity helpers.
//!
//! Extracted from `server/mod.rs` (C1 step 2). All items are scoped to
//! `pub(super)` because they are consumed by sibling modules
//! (`daq_server::ControlService` for the `metadata_*` and `spectrum_payload_*`
//! helpers, `startup::start_server_with_hardware` for the image-pipeline
//! helpers) and the test module.

use std::collections::HashMap;
use std::sync::Arc;

use common::core::Measurement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ImageSequenceFault {
    pub(super) previous_frame_number: u64,
    pub(super) current_frame_number: u64,
    pub(super) missing_frames: u64,
}

pub(super) fn spectrum_payload_from_parts(
    name: String,
    wavelengths: Vec<f64>,
    intensities: Vec<f64>,
    wavelength_unit: Option<String>,
    intensity_unit: Option<String>,
    metadata: Option<serde_json::Value>,
    timestamp_ns: u64,
) -> crate::grpc::proto::SpectrumPayload {
    let metadata_object = metadata.as_ref().and_then(serde_json::Value::as_object);
    let metadata_json = metadata
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_default();

    let merged = metadata_bool(metadata_object, &["merged"]).unwrap_or_else(|| {
        metadata_string(metadata_object, &["kind"]).is_some_and(|kind| kind == "echelle_merged")
            || name.contains("merged")
    });
    let order_index = metadata_i32(metadata_object, &["order_index", "relative_index"])
        .unwrap_or(if merged { -1 } else { 0 });

    crate::grpc::proto::SpectrumPayload {
        wavelengths,
        intensities,
        ivar: metadata_f64_list(metadata_object, &["ivar"]).unwrap_or_default(),
        wavelength_unit: wavelength_unit.unwrap_or_default(),
        order_index,
        merged,
        calibration_profile_id: metadata_string(
            metadata_object,
            &["calibration_profile_id", "profile_id"],
        )
        .unwrap_or_default(),
        quality_flags: metadata_string_list(metadata_object, &["quality_flags"])
            .unwrap_or_default(),
        snr_estimate: metadata_f64(metadata_object, &["snr_estimate"]).unwrap_or_default(),
        name,
        timestamp_ns,
        intensity_unit: intensity_unit.unwrap_or_default(),
        metadata_json,
        device_id: metadata_string(metadata_object, &["device_id"]).unwrap_or_default(),
    }
}

pub(super) fn metadata_string(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    })
}

fn metadata_bool(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<bool> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_bool)
    })
}

fn metadata_i32(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<i32> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
    })
}

fn metadata_f64(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<f64> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_f64)
    })
}

fn metadata_string_list(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<Vec<String>> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
    })
}

fn metadata_f64_list(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<Vec<f64>> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_f64)
                    .collect()
            })
    })
}

pub(super) fn encode_measurement_frame(
    measurement: &Measurement,
) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(measurement)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: serialized measurement payload is well under u32::MAX bytes
    let payload_len = payload.len() as u32;
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub(super) fn build_image_measurement(device_id: &str, frame: &common::data::Frame) -> Measurement {
    let buffer = match frame.bit_depth {
        16 => {
            if let Some(slice) = frame.as_u16_slice() {
                common::core::PixelBuffer::U16(slice.to_vec())
            } else {
                common::core::PixelBuffer::U8(frame.data.to_vec())
            }
        }
        _ => common::core::PixelBuffer::U8(frame.data.to_vec()),
    };

    let frame_metadata = frame.metadata.as_deref();
    let roi_origin = if frame.roi_x == 0 && frame.roi_y == 0 {
        None
    } else {
        Some((frame.roi_x, frame.roi_y))
    };

    Measurement::Image {
        name: device_id.to_string(),
        width: frame.width,
        height: frame.height,
        buffer,
        unit: "counts".to_string(),
        metadata: common::core::ImageMetadata {
            exposure_ms: frame.exposure_ms,
            gain: None,
            binning: frame_metadata
                .and_then(|meta| meta.binning)
                .map(|(x, y)| (u32::from(x), u32::from(y))),
            temperature_c: frame_metadata.and_then(|meta| meta.temperature_c),
            hardware_timestamp_us: None,
            readout_ms: None,
            roi_origin,
            frame_number: Some(frame.frame_number),
            sequence_gap_from_previous: None,
            summing_count: frame_metadata.and_then(|meta| meta.summing_count),
        },
        timestamp: chrono::Utc::now(),
    }
}

pub(super) fn load_echelle_profile_for_device(
    registry: &hardware::registry::DeviceRegistry,
    device_id: &str,
) -> Option<Arc<echelle::EchelleCalibrationProfile>> {
    let profile_path = registry.get_driver_config_string(device_id, "echelle_profile_path")?;
    match echelle::EchelleCalibrationProfile::load_from_path(std::path::Path::new(&profile_path)) {
        Ok(profile) => Some(Arc::new(profile)),
        Err(error) => {
            tracing::warn!(
                device_id = %device_id,
                profile_path = %profile_path,
                error = %error,
                "Failed to load echelle profile; server-side extraction disabled for device"
            );
            None
        }
    }
}

fn detect_image_sequence_fault(
    previous_frame_number: Option<u64>,
    current_frame_number: u64,
) -> Option<ImageSequenceFault> {
    let previous_frame_number = previous_frame_number?;
    if current_frame_number > previous_frame_number.saturating_add(1) {
        return Some(ImageSequenceFault {
            previous_frame_number,
            current_frame_number,
            missing_frames: current_frame_number - previous_frame_number - 1,
        });
    }
    None
}

pub(super) fn annotate_measurement_data_integrity(
    measurement: &mut Measurement,
    last_frame_numbers: &mut HashMap<String, u64>,
) -> Option<(String, ImageSequenceFault)> {
    let Measurement::Image { name, metadata, .. } = measurement else {
        return None;
    };
    let current_frame_number = metadata.frame_number?;
    let previous_frame_number = last_frame_numbers.get(name).copied();
    let fault = detect_image_sequence_fault(previous_frame_number, current_frame_number);

    if let Some(fault) = fault {
        metadata.sequence_gap_from_previous = Some(fault.missing_frames);
    }

    let next_frame_number = previous_frame_number.map_or(current_frame_number, |previous| {
        previous.max(current_frame_number)
    });
    last_frame_numbers.insert(name.clone(), next_frame_number);

    fault.map(|fault| (name.clone(), fault))
}

pub(super) fn log_data_integrity_fault(source: &str, fault: ImageSequenceFault) {
    tracing::warn!(
        target: "data_integrity",
        source,
        previous_frame_number = fault.previous_frame_number,
        current_frame_number = fault.current_frame_number,
        missing_frames = fault.missing_frames,
        "DataIntegrityFault: {} dropped {} frame(s) (prev={}, current={})",
        source,
        fault.missing_frames,
        fault.previous_frame_number,
        fault.current_frame_number
    );
}
