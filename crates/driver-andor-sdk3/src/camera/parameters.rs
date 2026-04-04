//! Parameter hardware callback attachment and dynamic feature registration.
//!
//! # Canonical `Parameter<T>` Registration Pattern
//!
//! Parameters are registered into a [`ParameterSet`] using the following pattern:
//!
//! 1. **Construct** a typed `Parameter<T>` with a name and default value:
//!    ```rust,ignore
//!    let mut param = Parameter::new("FeatureName".to_string(), 0.0f64)
//!        .with_description("Human-readable description");
//!    ```
//!
//! 2. **Attach metadata** (range, choices, dtype, read-only flag) as needed:
//!    ```rust,ignore
//!    param = param.with_range_introspectable(min, max);
//!    param = param.read_only(); // if the hardware feature is not writable
//!    ```
//!
//! 3. **Connect a `hardware_writer` callback** (hardware mode only) so that
//!    writing the parameter propagates the value to the physical device:
//!    ```rust,ignore
//!    param.connect_to_hardware_write(move |val: f64| {
//!        let fname = fname.clone();
//!        Box::pin(async move {
//!            // Call the SDK / FFI layer here, e.g.:
//!            AndorCamera::set_float_feature(handle, &fname, val).await
//!        })
//!    });
//!    ```
//!
//! 4. **Register** the fully-configured parameter into the set:
//!    ```rust,ignore
//!    params.register(param);
//!    ```
//!
//! The `hardware_writer` closure is **only wired in hardware mode** (guarded by
//! `#[cfg(feature = "camera")]`). In mock/test mode the parameter still exists
//! and holds state in memory, but writes are no-ops at the hardware boundary.

use super::AndorCamera;
#[cfg(feature = "camera")]
use super::{pause_apply_restart, sdk_blocking};
use crate::types::{ElectronicShutteringMode, GateMode, TriggerMode};
use common::core::Roi;
use common::observable::ParameterSet;
use common::parameter::Parameter;
#[cfg(feature = "camera")]
use std::sync::Arc;
#[cfg(feature = "camera")]
use std::sync::atomic::AtomicBool;

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
                                    crate::ffi_timeout::ffi_call_daq(
                                        move || AndorCamera::set_float_feature(handle, &fname, val),
                                        crate::ffi_timeout::FFI_CONFIG_TIMEOUT,
                                        "dynamic:set_float",
                                    )
                                    .await
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
                                    crate::ffi_timeout::ffi_call_daq(
                                        move || AndorCamera::set_int_feature(handle, &fname, val),
                                        crate::ffi_timeout::FFI_CONFIG_TIMEOUT,
                                        "dynamic:set_int",
                                    )
                                    .await
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
                                    crate::ffi_timeout::ffi_call_daq(
                                        move || AndorCamera::set_bool_feature(handle, &fname, val),
                                        crate::ffi_timeout::FFI_CONFIG_TIMEOUT,
                                        "dynamic:set_bool",
                                    )
                                    .await
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
                                    crate::ffi_timeout::ffi_call_daq(
                                        move || AndorCamera::set_enum_feature(handle, &fname, &val),
                                        crate::ffi_timeout::FFI_CONFIG_TIMEOUT,
                                        "dynamic:set_enum",
                                    )
                                    .await
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
        sdk_buffers: Arc<std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>>,
    ) {
        param.connect_to_hardware_write(move |val: f64| {
            let streaming = streaming.clone();
            let sdk_buffers = sdk_buffers.clone();
            Box::pin(sdk_blocking(move || {
                let result = AndorCamera::set_float_feature(handle, "ExposureTime", val);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                tracing::info!("Pausing acquisition to change ExposureTime");
                let bufs = sdk_buffers.lock().unwrap_or_else(|e| e.into_inner());
                pause_apply_restart(handle, &bufs, || {
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
        sdk_buffers: Arc<std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>>,
    ) {
        param.connect_to_hardware_write(move |gain: u32| {
            let streaming = streaming.clone();
            let sdk_buffers = sdk_buffers.clone();
            Box::pin(sdk_blocking(move || {
                let result = AndorCamera::set_int_feature(handle, "MCPGain", gain as i64);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                tracing::info!("Pausing acquisition to change MCPGain");
                let bufs = sdk_buffers.lock().unwrap_or_else(|e| e.into_inner());
                pause_apply_restart(handle, &bufs, || {
                    AndorCamera::set_int_feature(handle, "MCPGain", gain as i64)
                })
            }))
        });
    }

    /// DDG delay callback with pause-apply-restart support (bd-zg9e.1).
    ///
    /// Ensures DDGOutputSelector is set to "Gater" before applying the delay,
    /// since the SDK3 DDG output features are per-output.
    #[cfg(feature = "camera")]
    pub(super) fn attach_ddg_delay_callback(
        param: &mut Parameter<u64>,
        handle: AT_H,
        streaming: Arc<AtomicBool>,
        sdk_buffers: Arc<std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>>,
    ) {
        param.connect_to_hardware_write(move |delay_ps: u64| {
            let streaming = streaming.clone();
            let sdk_buffers = sdk_buffers.clone();
            Box::pin(sdk_blocking(move || {
                let _ = AndorCamera::set_enum_feature(handle, "DDGOutputSelector", "Gater");
                let delay_s = delay_ps as f64 * 1e-12;
                let result = AndorCamera::set_float_feature(handle, "DDGOutputDelay", delay_s);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                tracing::info!("Pausing acquisition to change DDGOutputDelay");
                let bufs = sdk_buffers.lock().unwrap_or_else(|e| e.into_inner());
                pause_apply_restart(handle, &bufs, || {
                    AndorCamera::set_float_feature(handle, "DDGOutputDelay", delay_s)
                })
            }))
        });
    }

    /// DDG width callback with pause-apply-restart support (bd-zg9e.1).
    #[cfg(feature = "camera")]
    pub(super) fn attach_ddg_width_callback(
        param: &mut Parameter<u64>,
        handle: AT_H,
        streaming: Arc<AtomicBool>,
        sdk_buffers: Arc<std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>>,
    ) {
        param.connect_to_hardware_write(move |width_ps: u64| {
            let streaming = streaming.clone();
            let sdk_buffers = sdk_buffers.clone();
            Box::pin(sdk_blocking(move || {
                let _ = AndorCamera::set_enum_feature(handle, "DDGOutputSelector", "Gater");
                let width_s = width_ps as f64 * 1e-12;
                let result = AndorCamera::set_float_feature(handle, "DDGOutputWidth", width_s);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                tracing::info!("Pausing acquisition to change DDGOutputWidth");
                let bufs = sdk_buffers.lock().unwrap_or_else(|e| e.into_inner());
                pause_apply_restart(handle, &bufs, || {
                    AndorCamera::set_float_feature(handle, "DDGOutputWidth", width_s)
                })
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

    // =========================================================================
    // bd-zg9e: New iStar intensifier and metadata callbacks
    // =========================================================================

    /// MCPIntelligate: simultaneous photocathode + MCP gating (bd-zg9e.2).
    #[cfg(feature = "camera")]
    pub(super) fn attach_mcp_intelligate_callback(
        param: &mut Parameter<bool>,
        handle: AT_H,
        streaming: Arc<AtomicBool>,
        sdk_buffers: Arc<std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>>,
    ) {
        param.connect_to_hardware_write(move |val: bool| {
            let streaming = streaming.clone();
            let sdk_buffers = sdk_buffers.clone();
            Box::pin(sdk_blocking(move || {
                let result = AndorCamera::set_bool_feature(handle, "MCPIntelligentGating", val);
                if result.is_ok() || !streaming.load(std::sync::atomic::Ordering::Relaxed) {
                    return result;
                }
                tracing::info!("Pausing acquisition to change MCPIntelligate");
                let bufs = sdk_buffers.lock().unwrap_or_else(|e| e.into_inner());
                pause_apply_restart(handle, &bufs, || {
                    AndorCamera::set_bool_feature(handle, "MCPIntelligentGating", val)
                })
            }))
        });
    }

    /// MCPVoltage: read-only monitoring of actual MCP high voltage (bd-zg9e.4).
    #[cfg(feature = "camera")]
    pub(super) fn attach_mcp_voltage_reader(param: &mut Parameter<u32>, handle: AT_H) {
        param.connect_to_hardware_read(move || {
            Box::pin(sdk_blocking(move || {
                AndorCamera::get_int_feature(handle, "MCPVoltage").map(|v| v as u32)
            }))
        });
    }

    /// InsertionDelay: gate latency control (bd-zg9e.5).
    #[cfg(feature = "camera")]
    pub(super) fn attach_insertion_delay_callback(
        param: &mut Parameter<crate::types::InsertionDelay>,
        handle: AT_H,
    ) {
        param.connect_to_hardware_write(move |mode: crate::types::InsertionDelay| {
            let mode_str = mode.to_string();
            Box::pin(sdk_blocking(move || {
                AndorCamera::set_enum_feature(handle, "InsertionDelay", &mode_str)
            }))
        });
    }

    /// Generic metadata boolean callback (bd-zg9e.6).
    ///
    /// Used for MetadataDDGInfo, MetadataMCPGain, MetadataFrameInfo.
    #[cfg(feature = "camera")]
    pub(super) fn attach_metadata_bool_callback(
        param: &mut Parameter<bool>,
        handle: AT_H,
        feature_name: &str,
    ) {
        let fname = feature_name.to_string();
        param.connect_to_hardware_write(move |val: bool| {
            let fname = fname.clone();
            Box::pin(sdk_blocking(move || {
                AndorCamera::set_bool_feature(handle, &fname, val)
            }))
        });
    }

    /// CameraAcquiring: read-only acquisition status (bd-zg9e.7).
    #[cfg(feature = "camera")]
    pub(super) fn attach_camera_acquiring_reader(param: &mut Parameter<bool>, handle: AT_H) {
        param.connect_to_hardware_read(move || {
            Box::pin(sdk_blocking(move || {
                AndorCamera::get_bool_feature(handle, "CameraAcquiring")
            }))
        });
    }

    /// BaselineLevel: read-only electronic baseline (bd-zg9e.8).
    #[cfg(feature = "camera")]
    pub(super) fn attach_baseline_level_reader(param: &mut Parameter<i64>, handle: AT_H) {
        param.connect_to_hardware_read(move || {
            Box::pin(sdk_blocking(move || {
                AndorCamera::get_int_feature(handle, "BaselineLevel")
            }))
        });
    }
}
