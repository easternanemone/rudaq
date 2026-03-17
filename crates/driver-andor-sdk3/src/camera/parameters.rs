//! Parameter hardware callback attachment and dynamic feature registration.

use super::AndorCamera;
#[cfg(feature = "camera")]
use super::{pause_apply_restart, sdk_blocking};
use crate::types::{ElectronicShutteringMode, GateMode, TriggerMode};
use common::core::Roi;
use common::error::DaqError;
use common::observable::ParameterSet;
use common::parameter::Parameter;
#[cfg(feature = "camera")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "camera")]
use std::sync::Arc;

#[cfg(feature = "camera")]
use andor_sdk3_sys::AT_H;

impl AndorCamera {
    /// Register dynamic SDK3 feature parameters from introspection results.
    ///
    /// For each discovered feature NOT already covered by core typed parameters
    /// (see [`CORE_FEATURE_NAMES`](super::CORE_FEATURE_NAMES)), creates a `Parameter<T>` with the appropriate
    /// type, introspectable metadata (ranges, enum values), and — in hardware
    /// mode — an SDK write callback via `spawn_blocking`.
    ///
    /// Feature type mapping:
    /// - `Float` → `Parameter<f64>` with `with_range_introspectable`
    /// - `Int`   → `Parameter<i64>` with `with_range_introspectable`
    /// - `Bool`  → `Parameter<bool>` with `dtype = "bool"`
    /// - `Enum`  → `Parameter<String>` with `with_choices_introspectable`
    /// - `Str`   → `Parameter<String>` with `dtype = "string"` (typically read-only)
    /// - `Command` → skipped (handled by `Commandable`/`Triggerable` traits)
    pub(super) fn register_dynamic_features(
        features: &[crate::introspection::DiscoveredFeature],
        params: &mut ParameterSet,
        handle: i32,
    ) {
        use crate::introspection::FeatureType;

        let mut count = 0u32;

        for feat in features {
            if !feat.is_displayable() || super::CORE_FEATURE_NAMES.contains(&feat.name.as_str()) {
                continue;
            }
            if feat.feature_type == FeatureType::Command {
                continue;
            }

            match feat.feature_type {
                FeatureType::Float => {
                    let mut param = Parameter::new(feat.name.clone(), 0.0f64)
                        .with_description(format!("SDK3: {}", feat.name));
                    if let Some((min, max)) = feat.float_range {
                        param = param.with_range_introspectable(min, max);
                    } else {
                        param = param.with_dtype("float");
                    }
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: f64| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_float_feature(handle, &fname, val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Int => {
                    let mut param = Parameter::new(feat.name.clone(), 0i64)
                        .with_description(format!("SDK3: {}", feat.name));
                    if let Some((min, max)) = feat.int_range {
                        param = param.with_range_introspectable(min, max);
                    } else {
                        param = param.with_dtype("int");
                    }
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: i64| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_int_feature(handle, &fname, val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Bool => {
                    let mut param = Parameter::new(feat.name.clone(), false)
                        .with_description(format!("SDK3: {}", feat.name))
                        .with_dtype("bool");
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: bool| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_bool_feature(handle, &fname, val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Enum => {
                    let default_val = feat.enum_values.first().cloned().unwrap_or_default();
                    let mut param = Parameter::new(feat.name.clone(), default_val)
                        .with_description(format!("SDK3: {}", feat.name));
                    if !feat.enum_values.is_empty() {
                        param = param.with_choices_introspectable(feat.enum_values.clone());
                    }
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: String| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_enum_feature(handle, &fname, &val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Str => {
                    let mut param = Parameter::new(feat.name.clone(), String::new())
                        .with_description(format!("SDK3: {}", feat.name))
                        .with_dtype("string");
                    if !feat.writable {
                        param = param.read_only();
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    // SDK3 string features are typically read-only identity fields
                    // (CameraModel, SerialNumber, etc.) — no hardware write callbacks.
                    params.register(param);
                }

                FeatureType::Command => unreachable!("Commands filtered above"),
            }

            count += 1;
        }

        tracing::info!(count, "Registered dynamic SDK3 feature parameters");
    }

    // =========================================================================
    // Parameter<T> hardware callback attachment methods
    // =========================================================================
    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
    // It does not serialize concurrent calls.
    //
    // Exposure and MCP gain callbacks use pause_apply_restart (bd-4msn) to
    // handle the case where the SDK rejects parameter changes during active
    // acquisition (AT_ERR_COMM). The callback tries the SDK call directly first;
    // on failure while streaming, it stops acquisition, applies the change,
    // and restarts without flushing buffers.

    #[cfg(feature = "camera")]
    pub(super) fn attach_exposure_callback(
        param: &mut Parameter<f64>,
        handle: AT_H,
        streaming: Arc<AtomicBool>,
    ) {
        param.connect_to_hardware_write(move |val: f64| {
            let streaming = streaming.clone();
            Box::pin(sdk_blocking(move || {
                let result = AndorCamera::set_float_feature(handle, "ExposureTime", val);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                // Streaming and failed — pause acquisition, apply, restart (bd-4msn)
                tracing::info!("Pausing acquisition to change ExposureTime");
                pause_apply_restart(handle, || {
                    AndorCamera::set_float_feature(handle, "ExposureTime", val)
                })
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_trigger_mode_callback(param: &mut Parameter<TriggerMode>, handle: AT_H) {
        param.connect_to_hardware_write(move |mode: TriggerMode| {
            let mode_str = mode.to_string();
            Box::pin(sdk_blocking(move || {
                AndorCamera::set_enum_feature(handle, "TriggerMode", &mode_str)
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_gate_mode_callback(param: &mut Parameter<GateMode>, handle: AT_H) {
        param.connect_to_hardware_write(move |mode: GateMode| {
            let mode_str = mode.to_string();
            Box::pin(sdk_blocking(move || {
                AndorCamera::set_enum_feature(handle, "GateMode", &mode_str)?;
                // When DDG mode is active, select the Gater (MCP intensifier)
                // as the DDG output target so that DDGOutputDelay/Width
                // control the MCP gate timing.
                if mode == GateMode::DDG {
                    AndorCamera::set_enum_feature(handle, "DDGOutputSelector", "Gater")?;
                }
                Ok(())
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_mcp_gain_callback(
        param: &mut Parameter<u32>,
        handle: AT_H,
        streaming: Arc<AtomicBool>,
    ) {
        param.connect_to_hardware_write(move |gain: u32| {
            let streaming = streaming.clone();
            Box::pin(sdk_blocking(move || {
                let result = AndorCamera::set_int_feature(handle, "MCPGain", gain as i64);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                // Streaming and failed — pause acquisition, apply, restart (bd-4msn)
                tracing::info!("Pausing acquisition to change MCPGain");
                pause_apply_restart(handle, || {
                    AndorCamera::set_int_feature(handle, "MCPGain", gain as i64)
                })
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_ddg_delay_callback(param: &mut Parameter<u64>, handle: AT_H) {
        param.connect_to_hardware_write(move |delay_ps: u64| {
            Box::pin(sdk_blocking(move || {
                // SDK3 DDGOutputDelay is in seconds; parameter stores picoseconds
                AndorCamera::set_float_feature(handle, "DDGOutputDelay", delay_ps as f64 * 1e-12)
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_ddg_width_callback(param: &mut Parameter<u64>, handle: AT_H) {
        param.connect_to_hardware_write(move |width_ps: u64| {
            Box::pin(sdk_blocking(move || {
                // SDK3 DDGOutputWidth is in seconds; parameter stores picoseconds
                AndorCamera::set_float_feature(handle, "DDGOutputWidth", width_ps as f64 * 1e-12)
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_temperature_reader(param: &mut Parameter<f64>, handle: AT_H) {
        param.connect_to_hardware_read(move || {
            Box::pin(sdk_blocking(move || {
                AndorCamera::get_float_feature(handle, "SensorTemperature")
            }))
        });
    }

    /// Read `TargetSensorTemperature` from the SDK.
    ///
    /// On cameras with `TemperatureControl` (iStar, Zyla, Marana), this value
    /// is managed by the SDK and reflects the selected calibrated setpoint.
    /// It is read-only — use `TemperatureControl` enum to change the target.
    #[cfg(feature = "camera")]
    pub(super) fn attach_target_temperature_reader(param: &mut Parameter<f64>, handle: AT_H) {
        param.connect_to_hardware_read(move || {
            Box::pin(sdk_blocking(move || {
                AndorCamera::get_float_feature(handle, "TargetSensorTemperature")
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_roi_callback(param: &mut Parameter<Roi>, handle: AT_H) {
        param.connect_to_hardware_write(move |roi: Roi| {
            Box::pin(sdk_blocking(move || {
                // SDK3 AOI features use 1-based coordinates
                AndorCamera::set_int_feature(handle, "AOILeft", roi.x as i64 + 1)?;
                AndorCamera::set_int_feature(handle, "AOITop", roi.y as i64 + 1)?;
                AndorCamera::set_int_feature(handle, "AOIWidth", roi.width as i64)?;
                AndorCamera::set_int_feature(handle, "AOIHeight", roi.height as i64)?;
                Ok(())
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_binning_callback(param: &mut Parameter<(u32, u32)>, handle: AT_H) {
        param.connect_to_hardware_write(move |bin: (u32, u32)| {
            Box::pin(sdk_blocking(move || {
                AndorCamera::set_int_feature(handle, "AOIHBin", bin.0 as i64)?;
                AndorCamera::set_int_feature(handle, "AOIVBin", bin.1 as i64)?;
                Ok(())
            }))
        });
    }

    #[cfg(feature = "camera")]
    pub(super) fn attach_electronic_shuttering_callback(
        param: &mut Parameter<ElectronicShutteringMode>,
        handle: AT_H,
    ) {
        param.connect_to_hardware_write(move |mode: ElectronicShutteringMode| {
            let mode_str = mode.to_string();
            Box::pin(sdk_blocking(move || {
                AndorCamera::set_enum_feature(handle, "ElectronicShutteringMode", &mode_str)
            }))
        });
    }
}
