//! Dynamic SDK3 feature discovery and introspection.
//!
//! Enumerates all features available on a camera handle, probing their types,
//! ranges, and enum values. This enables dynamic `Parameter<T>` creation and
//! Database metadata caching without hardcoding feature sets.
//!
//! # Hardware vs Mock
//!
//! - With `#[cfg(feature = "camera")]`: Calls real SDK3 FFI functions to probe
//!   the hardware for implemented features, ranges, and enum values.
//! - Without: Returns a hardcoded set of realistic iStar features for
//!   development and testing.

use std::fmt;

/// The data type of an SDK3 feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureType {
    /// 64-bit integer (`AT_GetInt` / `AT_SetInt`)
    Int,
    /// Double-precision float (`AT_GetFloat` / `AT_SetFloat`)
    Float,
    /// Boolean (`AT_GetBool` / `AT_SetBool`)
    Bool,
    /// Enumerated choice (`AT_GetEnumIndex` / `AT_SetEnumString`)
    Enum,
    /// String value (`AT_GetString` / `AT_SetString`)
    Str,
    /// Write-only command (`AT_Command`)
    Command,
}

impl fmt::Display for FeatureType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::Enum => "enum",
            Self::Str => "string",
            Self::Command => "command",
        })
    }
}

/// A feature discovered by probing the SDK3 or from the mock catalog.
#[derive(Debug, Clone)]
pub struct DiscoveredFeature {
    /// SDK3 feature name (e.g., "ExposureTime").
    pub name: String,
    /// Data type of this feature.
    pub feature_type: FeatureType,
    /// Whether the feature is implemented on this camera model.
    pub implemented: bool,
    /// Whether the feature can be read.
    pub readable: bool,
    /// Whether the feature can be written.
    pub writable: bool,
    /// Integer range (min, max) — only for `Int` features.
    pub int_range: Option<(i64, i64)>,
    /// Float range (min, max) — only for `Float` features.
    pub float_range: Option<(f64, f64)>,
    /// Enum choice strings — only for `Enum` features.
    pub enum_values: Vec<String>,
    /// UI grouping category (e.g., "Intensifier", "Acquisition").
    pub group: Option<String>,
}

impl DiscoveredFeature {
    /// Returns `true` if this feature can be displayed in a UI panel.
    pub fn is_displayable(&self) -> bool {
        self.implemented && (self.readable || self.feature_type == FeatureType::Command)
    }

    /// Returns `true` if this feature can be edited by the user.
    pub fn is_editable(&self) -> bool {
        self.implemented && self.writable
    }

    /// Construct a feature that is not implemented on this camera model.
    fn not_implemented(name: &str, feature_type: FeatureType, group: Option<&str>) -> Self {
        Self {
            name: name.into(),
            feature_type,
            implemented: false,
            readable: false,
            writable: false,
            int_range: None,
            float_range: None,
            enum_values: vec![],
            group: group.map(Into::into),
        }
    }

    /// Builder method to set the UI group on a feature.
    fn with_group(mut self, group: &str) -> Self {
        self.group = Some(group.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Static feature-to-type mapping table
// ---------------------------------------------------------------------------

/// Compile-time entry mapping a feature name to its type and UI group.
struct FeatureEntry {
    name: &'static str,
    feature_type: FeatureType,
    group: Option<&'static str>,
}

/// All known SDK3 features with their types and groups.
///
/// Sourced from the Andor SDK3 Software Development Kit manual, Feature Reference.
const FEATURE_TABLE: &[FeatureEntry] = &[
    // ── Sensor / ROI ─────────────────────────────────────────────
    FeatureEntry {
        name: "SensorWidth",
        feature_type: FeatureType::Int,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "SensorHeight",
        feature_type: FeatureType::Int,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "AOIWidth",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "AOIHeight",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "AOILeft",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "AOITop",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "AOIStride",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "AOIHBin",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "AOIVBin",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "ImageSizeBytes",
        feature_type: FeatureType::Int,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "FullAOIControl",
        feature_type: FeatureType::Bool,
        group: Some("ROI"),
    },
    FeatureEntry {
        name: "VerticallyCentreAOI",
        feature_type: FeatureType::Bool,
        group: Some("ROI"),
    },
    // ── Acquisition control ──────────────────────────────────────
    FeatureEntry {
        name: "ExposureTime",
        feature_type: FeatureType::Float,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "ReadoutTime",
        feature_type: FeatureType::Float,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "FrameRate",
        feature_type: FeatureType::Float,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "MaxInterfaceTransferRate",
        feature_type: FeatureType::Float,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "TriggerMode",
        feature_type: FeatureType::Enum,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "CycleMode",
        feature_type: FeatureType::Enum,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "FrameCount",
        feature_type: FeatureType::Int,
        group: Some("Acquisition"),
    },
    FeatureEntry {
        name: "AccumulateCount",
        feature_type: FeatureType::Int,
        group: Some("Acquisition"),
    },
    // ── Readout ──────────────────────────────────────────────────
    FeatureEntry {
        name: "PixelEncoding",
        feature_type: FeatureType::Enum,
        group: Some("Readout"),
    },
    FeatureEntry {
        name: "PixelReadoutRate",
        feature_type: FeatureType::Enum,
        group: Some("Readout"),
    },
    FeatureEntry {
        name: "ElectronicShutteringMode",
        feature_type: FeatureType::Enum,
        group: Some("Readout"),
    },
    FeatureEntry {
        name: "SimplePreAmpGainControl",
        feature_type: FeatureType::Enum,
        group: Some("Readout"),
    },
    FeatureEntry {
        name: "BytesPerPixel",
        feature_type: FeatureType::Float,
        group: Some("Readout"),
    },
    FeatureEntry {
        name: "PixelWidth",
        feature_type: FeatureType::Float,
        group: Some("Readout"),
    },
    FeatureEntry {
        name: "PixelHeight",
        feature_type: FeatureType::Float,
        group: Some("Readout"),
    },
    // ── Sensor / Temperature ─────────────────────────────────────
    FeatureEntry {
        name: "SensorTemperature",
        feature_type: FeatureType::Float,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "TargetSensorTemperature",
        feature_type: FeatureType::Float,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "SensorCooling",
        feature_type: FeatureType::Bool,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "TemperatureStatus",
        feature_type: FeatureType::Enum,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "TemperatureControl",
        feature_type: FeatureType::Enum,
        group: Some("Sensor"),
    },
    FeatureEntry {
        name: "FanSpeed",
        feature_type: FeatureType::Enum,
        group: Some("Sensor"),
    },
    // ── Intensifier (iStar-specific) ─────────────────────────────
    FeatureEntry {
        name: "MCPGain",
        feature_type: FeatureType::Int,
        group: Some("Intensifier"),
    },
    FeatureEntry {
        name: "GateMode",
        feature_type: FeatureType::Enum,
        group: Some("Intensifier"),
    },
    FeatureEntry {
        name: "MCPIntelligentGating",
        feature_type: FeatureType::Bool,
        group: Some("Intensifier"),
    },
    FeatureEntry {
        name: "InsertionDelay",
        feature_type: FeatureType::Enum,
        group: Some("Intensifier"),
    },
    // ── DDG Timing ───────────────────────────────────────────────
    FeatureEntry {
        name: "DDGOutputDelay",
        feature_type: FeatureType::Float,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGOutputWidth",
        feature_type: FeatureType::Float,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGOutputSelector",
        feature_type: FeatureType::Enum,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGStepEnabled",
        feature_type: FeatureType::Bool,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGStepCount",
        feature_type: FeatureType::Int,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGStepDelayCoefficient",
        feature_type: FeatureType::Float,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGStepWidthCoefficient",
        feature_type: FeatureType::Float,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGStepUploadProgress",
        feature_type: FeatureType::Float,
        group: Some("Timing"),
    },
    FeatureEntry {
        name: "DDGStepUploadRequired",
        feature_type: FeatureType::Bool,
        group: Some("Timing"),
    },
    // ── Metadata / Timestamping ──────────────────────────────────
    FeatureEntry {
        name: "MetadataEnable",
        feature_type: FeatureType::Bool,
        group: Some("Metadata"),
    },
    FeatureEntry {
        name: "MetadataFrame",
        feature_type: FeatureType::Bool,
        group: Some("Metadata"),
    },
    FeatureEntry {
        name: "MetadataTimestamp",
        feature_type: FeatureType::Bool,
        group: Some("Metadata"),
    },
    FeatureEntry {
        name: "TimestampClock",
        feature_type: FeatureType::Int,
        group: Some("Metadata"),
    },
    FeatureEntry {
        name: "TimestampClockFrequency",
        feature_type: FeatureType::Int,
        group: Some("Metadata"),
    },
    FeatureEntry {
        name: "TimestampClockReset",
        feature_type: FeatureType::Command,
        group: Some("Metadata"),
    },
    // ── I/O Control ──────────────────────────────────────────────
    FeatureEntry {
        name: "IOSelector",
        feature_type: FeatureType::Enum,
        group: Some("I/O"),
    },
    FeatureEntry {
        name: "IOInvert",
        feature_type: FeatureType::Bool,
        group: Some("I/O"),
    },
    FeatureEntry {
        name: "AuxiliaryOutSource",
        feature_type: FeatureType::Enum,
        group: Some("I/O"),
    },
    FeatureEntry {
        name: "AuxOutSourceTwo",
        feature_type: FeatureType::Enum,
        group: Some("I/O"),
    },
    // ── Device Identity ──────────────────────────────────────────
    FeatureEntry {
        name: "CameraModel",
        feature_type: FeatureType::Str,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "SerialNumber",
        feature_type: FeatureType::Str,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "CameraName",
        feature_type: FeatureType::Str,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "FirmwareVersion",
        feature_type: FeatureType::Str,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "InterfaceType",
        feature_type: FeatureType::Enum,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "CameraFamily",
        feature_type: FeatureType::Enum,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "ControllerID",
        feature_type: FeatureType::Int,
        group: Some("Device"),
    },
    FeatureEntry {
        name: "DeviceCount",
        feature_type: FeatureType::Int,
        group: Some("Device"),
    },
    // ── Corrections ──────────────────────────────────────────────
    FeatureEntry {
        name: "SpuriousNoiseFilter",
        feature_type: FeatureType::Bool,
        group: Some("Corrections"),
    },
    FeatureEntry {
        name: "StaticBlemishCorrection",
        feature_type: FeatureType::Bool,
        group: Some("Corrections"),
    },
    FeatureEntry {
        name: "Overlap",
        feature_type: FeatureType::Bool,
        group: Some("Corrections"),
    },
    FeatureEntry {
        name: "RollingShutterGlobalClear",
        feature_type: FeatureType::Bool,
        group: Some("Corrections"),
    },
    // ── Commands ─────────────────────────────────────────────────
    FeatureEntry {
        name: "AcquisitionStart",
        feature_type: FeatureType::Command,
        group: None,
    },
    FeatureEntry {
        name: "AcquisitionStop",
        feature_type: FeatureType::Command,
        group: None,
    },
    FeatureEntry {
        name: "SoftwareTrigger",
        feature_type: FeatureType::Command,
        group: None,
    },
    FeatureEntry {
        name: "BufferOverflowEvent",
        feature_type: FeatureType::Command,
        group: None,
    },
    FeatureEntry {
        name: "EventsMissedEvent",
        feature_type: FeatureType::Command,
        group: None,
    },
    FeatureEntry {
        name: "EventEnable",
        feature_type: FeatureType::Bool,
        group: None,
    },
];

// =============================================================================
// FFI helpers (hardware only)
// =============================================================================

#[cfg(feature = "camera")]
use andor_sdk3_sys::{
    AT_BOOL, AT_FALSE, AT_GetEnumCount, AT_GetEnumStringByIndex, AT_GetFloatMax, AT_GetFloatMin,
    AT_GetIntMax, AT_GetIntMin, AT_H, AT_IsEnumIndexImplemented, AT_IsReadable, AT_SUCCESS,
};
#[cfg(feature = "camera")]
use andor_sdk3_sys::{from_wide_string, to_wide_string, wide_string_buffer};

#[cfg(feature = "camera")]
use crate::error::sdk_result;

/// Get the minimum value of an integer feature.
#[cfg(feature = "camera")]
pub fn get_int_min(handle: AT_H, feature: &str) -> anyhow::Result<i64> {
    unsafe {
        let feature_wide = to_wide_string(feature);
        let mut value: andor_sdk3_sys::AT_64 = 0;
        let ret = AT_GetIntMin(handle, feature_wide.as_ptr(), &mut value);
        sdk_result(ret)?;
        Ok(value)
    }
}

/// Get the maximum value of an integer feature.
#[cfg(feature = "camera")]
pub fn get_int_max(handle: AT_H, feature: &str) -> anyhow::Result<i64> {
    unsafe {
        let feature_wide = to_wide_string(feature);
        let mut value: andor_sdk3_sys::AT_64 = 0;
        let ret = AT_GetIntMax(handle, feature_wide.as_ptr(), &mut value);
        sdk_result(ret)?;
        Ok(value)
    }
}

/// Get the number of enum choices for a feature.
#[cfg(feature = "camera")]
pub fn get_enum_count(handle: AT_H, feature: &str) -> anyhow::Result<i32> {
    unsafe {
        let feature_wide = to_wide_string(feature);
        let mut count: std::os::raw::c_int = 0;
        let ret = AT_GetEnumCount(handle, feature_wide.as_ptr(), &mut count);
        sdk_result(ret)?;
        Ok(count)
    }
}

/// Get the string value of an enum choice by index.
#[cfg(feature = "camera")]
pub fn get_enum_string_by_index(handle: AT_H, feature: &str, index: i32) -> anyhow::Result<String> {
    unsafe {
        let feature_wide = to_wide_string(feature);
        let mut buffer = wide_string_buffer(256);
        let ret = AT_GetEnumStringByIndex(
            handle,
            feature_wide.as_ptr(),
            index,
            buffer.as_mut_ptr(),
            256,
        );
        sdk_result(ret)?;
        Ok(from_wide_string(&buffer))
    }
}

/// Check whether a feature is readable.
#[cfg(feature = "camera")]
pub fn is_feature_readable(handle: AT_H, feature: &str) -> anyhow::Result<bool> {
    unsafe {
        let feature_wide = to_wide_string(feature);
        let mut readable: AT_BOOL = AT_FALSE;
        let ret = AT_IsReadable(handle, feature_wide.as_ptr(), &mut readable);
        sdk_result(ret)?;
        Ok(readable != AT_FALSE)
    }
}

/// Check whether an enum index is implemented (available on this model).
#[cfg(feature = "camera")]
fn is_enum_index_implemented(handle: AT_H, feature: &str, index: i32) -> anyhow::Result<bool> {
    unsafe {
        let feature_wide = to_wide_string(feature);
        let mut implemented: AT_BOOL = AT_FALSE;
        let ret = AT_IsEnumIndexImplemented(handle, feature_wide.as_ptr(), index, &mut implemented);
        // Some cameras return AT_ERR_NOTIMPLEMENTED for the query itself
        if ret != AT_SUCCESS {
            return Ok(false);
        }
        Ok(implemented != AT_FALSE)
    }
}

// =============================================================================
// Hardware introspection
// =============================================================================

/// Probe all known SDK3 features on a live camera handle.
///
/// For each feature in the static table, checks `AT_IsImplemented`, then
/// probes ranges (int/float) or enum values depending on the feature type.
#[cfg(feature = "camera")]
pub fn introspect_all_features(handle: AT_H) -> Vec<DiscoveredFeature> {
    use crate::camera::AndorCamera;

    let mut features = Vec::with_capacity(FEATURE_TABLE.len());

    for entry in FEATURE_TABLE {
        let implemented = AndorCamera::is_feature_implemented(handle, entry.name).unwrap_or(false);

        if !implemented {
            features.push(DiscoveredFeature::not_implemented(
                entry.name,
                entry.feature_type,
                entry.group,
            ));
            continue;
        }

        let readable = is_feature_readable(handle, entry.name).unwrap_or(false);
        let writable = AndorCamera::is_feature_writable(handle, entry.name).unwrap_or(false);

        let mut int_range = None;
        let mut float_range = None;
        let mut enum_values = vec![];

        match entry.feature_type {
            FeatureType::Int => {
                if let (Ok(min), Ok(max)) = (
                    get_int_min(handle, entry.name),
                    get_int_max(handle, entry.name),
                ) {
                    int_range = Some((min, max));
                }
            }
            FeatureType::Float => {
                if let (Ok(min), Ok(max)) = (
                    AndorCamera::get_float_min(handle, entry.name),
                    AndorCamera::get_float_max(handle, entry.name),
                ) {
                    float_range = Some((min, max));
                }
            }
            FeatureType::Enum => {
                if let Ok(count) = get_enum_count(handle, entry.name) {
                    for i in 0..count {
                        // Only include choices that are implemented on this model
                        if is_enum_index_implemented(handle, entry.name, i).unwrap_or(false) {
                            if let Ok(s) = get_enum_string_by_index(handle, entry.name, i) {
                                enum_values.push(s);
                            }
                        }
                    }
                }
            }
            FeatureType::Bool | FeatureType::Str | FeatureType::Command => {}
        }

        features.push(DiscoveredFeature {
            name: entry.name.to_string(),
            feature_type: entry.feature_type,
            implemented: true,
            readable,
            writable,
            int_range,
            float_range,
            enum_values,
            group: entry.group.map(String::from),
        });
    }

    features
}

// =============================================================================
// Mock introspection (non-hardware builds)
// =============================================================================

/// Returns a hardcoded set of realistic iStar features for mock/test builds.
///
/// Includes all iStar-specific features (MCP gain, gate modes, DDG timing)
/// plus common camera features with representative ranges and enum values.
/// Always compiled (not gated by `camera` feature) because mock code must
/// remain functional alongside real SDK code for integration testing.
pub fn introspect_mock_features() -> Vec<DiscoveredFeature> {
    vec![
        // ── ROI ──────────────────────────────────────────────────
        df_int("SensorWidth", (1, 2048), false, "Sensor"),
        df_int("SensorHeight", (1, 2048), false, "Sensor"),
        df_int("AOIWidth", (1, 2048), true, "ROI"),
        df_int("AOIHeight", (1, 2048), true, "ROI"),
        df_int("AOILeft", (1, 2048), true, "ROI"),
        df_int("AOITop", (1, 2048), true, "ROI"),
        df_int("AOIStride", (1, 65536), false, "ROI"),
        df_int("AOIHBin", (1, 8), true, "ROI"),
        df_int("AOIVBin", (1, 8), true, "ROI"),
        df_int("ImageSizeBytes", (1, 67108864), false, "ROI"),
        df_bool("FullAOIControl", false, "ROI"),
        df_bool("VerticallyCentreAOI", true, "ROI"),
        // ── Acquisition ──────────────────────────────────────────
        df_float("ExposureTime", (1e-7, 30.0), true, "Acquisition"),
        df_float("ReadoutTime", (0.0, 1.0), false, "Acquisition"),
        df_float("FrameRate", (0.001, 1000.0), false, "Acquisition"),
        df_float(
            "MaxInterfaceTransferRate",
            (0.0, 10e9),
            false,
            "Acquisition",
        ),
        df_enum(
            "TriggerMode",
            &[
                "Internal",
                "External",
                "External Start",
                "External Exposure",
                "Software",
            ],
            true,
            "Acquisition",
        ),
        df_enum("CycleMode", &["Fixed", "Continuous"], true, "Acquisition"),
        df_int("FrameCount", (1, 1_000_000), true, "Acquisition"),
        df_int("AccumulateCount", (1, 65535), true, "Acquisition"),
        // ── Readout ──────────────────────────────────────────────
        df_enum(
            "PixelEncoding",
            &["Mono12", "Mono12Packed", "Mono16", "Mono32"],
            true,
            "Readout",
        ),
        df_enum(
            "PixelReadoutRate",
            &["100 MHz", "200 MHz", "280 MHz"],
            true,
            "Readout",
        ),
        df_enum(
            "ElectronicShutteringMode",
            &["Rolling", "Global"],
            true,
            "Readout",
        ),
        df_enum(
            "SimplePreAmpGainControl",
            &[
                "12-bit (low noise)",
                "12-bit (high well capacity)",
                "16-bit (low noise & high well capacity)",
            ],
            true,
            "Readout",
        ),
        df_float("BytesPerPixel", (1.0, 4.0), false, "Readout"),
        df_float("PixelWidth", (1.0, 100.0), false, "Readout"),
        df_float("PixelHeight", (1.0, 100.0), false, "Readout"),
        // ── Sensor / Temperature ─────────────────────────────────
        df_float("SensorTemperature", (-100.0, 50.0), false, "Sensor"),
        df_float("TargetSensorTemperature", (-40.0, 5.0), true, "Sensor"),
        df_bool("SensorCooling", true, "Sensor"),
        df_enum(
            "TemperatureStatus",
            &[
                "Cooler Off",
                "Stabilised",
                "Cooling",
                "Drift",
                "Not Stabilised",
                "Fault",
            ],
            false,
            "Sensor",
        ),
        df_enum("TemperatureControl", &["Off", "On"], true, "Sensor"),
        df_enum("FanSpeed", &["Off", "Low", "On"], true, "Sensor"),
        // ── Intensifier (iStar) ──────────────────────────────────
        df_int("MCPGain", (0, 4095), true, "Intensifier"),
        df_enum(
            "GateMode",
            &[
                "CW On",
                "CW Off",
                "Fire Only",
                "Gate Only",
                "Fire and Gate",
                "DDG",
            ],
            true,
            "Intensifier",
        ),
        df_bool("MCPIntelligentGating", true, "Intensifier"),
        df_enum("InsertionDelay", &["Normal", "Fast"], false, "Intensifier"),
        // ── DDG Timing ───────────────────────────────────────────
        df_float("DDGOutputDelay", (0.0, 100.0), true, "Timing"),
        df_float("DDGOutputWidth", (0.0, 100.0), true, "Timing"),
        df_enum(
            "DDGOutputSelector",
            &["Gater", "Fire 1", "Fire N"],
            true,
            "Timing",
        ),
        df_bool("DDGStepEnabled", true, "Timing"),
        df_int("DDGStepCount", (1, 1000), true, "Timing"),
        df_float("DDGStepDelayCoefficient", (0.0, 100.0), true, "Timing"),
        df_float("DDGStepWidthCoefficient", (0.0, 100.0), true, "Timing"),
        df_float("DDGStepUploadProgress", (0.0, 100.0), false, "Timing"),
        df_bool("DDGStepUploadRequired", false, "Timing"),
        // ── Metadata ─────────────────────────────────────────────
        df_bool("MetadataEnable", true, "Metadata"),
        df_bool("MetadataFrame", true, "Metadata"),
        df_bool("MetadataTimestamp", true, "Metadata"),
        df_int("TimestampClock", (0, i64::MAX), false, "Metadata"),
        df_int(
            "TimestampClockFrequency",
            (0, 1_000_000_000),
            false,
            "Metadata",
        ),
        // ── I/O ──────────────────────────────────────────────────
        df_enum(
            "IOSelector",
            &["Fire 1", "Fire N", "Aux Out 1", "Aux Out 2"],
            true,
            "I/O",
        ),
        df_bool("IOInvert", true, "I/O"),
        df_enum(
            "AuxiliaryOutSource",
            &["FireRow1", "FireRowN", "FireAll", "FireAny"],
            true,
            "I/O",
        ),
        df_enum(
            "AuxOutSourceTwo",
            &["FireRow1", "FireRowN", "FireAll", "FireAny"],
            true,
            "I/O",
        ),
        // ── Device Identity ──────────────────────────────────────
        df_str("CameraModel", false, "Device"),
        df_str("SerialNumber", false, "Device"),
        df_str("CameraName", true, "Device"),
        df_str("FirmwareVersion", false, "Device"),
        df_enum("InterfaceType", &["USB3", "CameraLink"], false, "Device"),
        df_enum("CameraFamily", &["iStar"], false, "Device"),
        df_int("ControllerID", (0, 255), false, "Device"),
        // DeviceCount is a system-handle feature, not per-camera
        DiscoveredFeature::not_implemented("DeviceCount", FeatureType::Int, Some("Device")),
        // ── Corrections ──────────────────────────────────────────
        df_bool("SpuriousNoiseFilter", true, "Corrections"),
        df_bool("StaticBlemishCorrection", true, "Corrections"),
        // Overlap and RollingShutterGlobalClear not available on iStar
        DiscoveredFeature::not_implemented("Overlap", FeatureType::Bool, Some("Corrections")),
        DiscoveredFeature::not_implemented(
            "RollingShutterGlobalClear",
            FeatureType::Bool,
            Some("Corrections"),
        ),
        // ── Commands ─────────────────────────────────────────────
        df_cmd("AcquisitionStart"),
        df_cmd("AcquisitionStop"),
        df_cmd("SoftwareTrigger"),
        df_cmd("TimestampClockReset").with_group("Metadata"),
        df_cmd_readonly("BufferOverflowEvent"),
        df_cmd_readonly("EventsMissedEvent"),
        df_bool("EventEnable", true, ""),
    ]
}

// Helper constructors for mock features to reduce verbosity

/// Convert a group string to `Option<String>`, treating empty as `None`.
fn group_option(group: &str) -> Option<String> {
    if group.is_empty() {
        None
    } else {
        Some(group.into())
    }
}

fn df_int(name: &str, range: (i64, i64), writable: bool, group: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Int,
        implemented: true,
        readable: true,
        writable,
        int_range: Some(range),
        float_range: None,
        enum_values: vec![],
        group: group_option(group),
    }
}

fn df_float(name: &str, range: (f64, f64), writable: bool, group: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Float,
        implemented: true,
        readable: true,
        writable,
        int_range: None,
        float_range: Some(range),
        enum_values: vec![],
        group: group_option(group),
    }
}

fn df_bool(name: &str, writable: bool, group: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Bool,
        implemented: true,
        readable: true,
        writable,
        int_range: None,
        float_range: None,
        enum_values: vec![],
        group: group_option(group),
    }
}

fn df_enum(name: &str, values: &[&str], writable: bool, group: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Enum,
        implemented: true,
        readable: true,
        writable,
        int_range: None,
        float_range: None,
        enum_values: values.iter().map(|s| (*s).to_string()).collect(),
        group: group_option(group),
    }
}

fn df_str(name: &str, writable: bool, group: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Str,
        implemented: true,
        readable: true,
        writable,
        int_range: None,
        float_range: None,
        enum_values: vec![],
        group: group_option(group),
    }
}

fn df_cmd(name: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Command,
        implemented: true,
        readable: false,
        writable: true,
        int_range: None,
        float_range: None,
        enum_values: vec![],
        group: None,
    }
}

/// Read-only event/command (implemented but not writable).
fn df_cmd_readonly(name: &str) -> DiscoveredFeature {
    DiscoveredFeature {
        name: name.into(),
        feature_type: FeatureType::Command,
        implemented: true,
        readable: false,
        writable: false,
        int_range: None,
        float_range: None,
        enum_values: vec![],
        group: None,
    }
}

// =============================================================================
// Convenience accessors
// =============================================================================

/// Returns the list of all known SDK3 feature names and their types.
///
/// This does not query hardware — it returns the compile-time table.
pub fn known_features() -> Vec<(&'static str, FeatureType, Option<&'static str>)> {
    FEATURE_TABLE
        .iter()
        .map(|e| (e.name, e.feature_type, e.group))
        .collect()
}

/// Returns all unique group names from the feature table.
pub fn known_groups() -> Vec<&'static str> {
    let mut groups: Vec<&str> = FEATURE_TABLE.iter().filter_map(|e| e.group).collect();
    groups.sort_unstable();
    groups.dedup();
    groups
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_table_has_expected_count() {
        assert!(
            FEATURE_TABLE.len() >= 60,
            "Expected at least 60 features, found {}",
            FEATURE_TABLE.len()
        );
    }

    #[test]
    fn feature_table_no_duplicate_names() {
        let mut names: Vec<&str> = FEATURE_TABLE.iter().map(|e| e.name).collect();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            original_len,
            "Feature table has duplicate names"
        );
    }

    #[test]
    fn feature_table_covers_all_types() {
        let types: std::collections::HashSet<FeatureType> =
            FEATURE_TABLE.iter().map(|e| e.feature_type).collect();
        assert!(types.contains(&FeatureType::Int));
        assert!(types.contains(&FeatureType::Float));
        assert!(types.contains(&FeatureType::Bool));
        assert!(types.contains(&FeatureType::Enum));
        assert!(types.contains(&FeatureType::Str));
        assert!(types.contains(&FeatureType::Command));
    }

    #[test]
    fn known_features_returns_all() {
        assert_eq!(known_features().len(), FEATURE_TABLE.len());
    }

    #[test]
    fn known_groups_are_sorted_and_unique() {
        let groups = known_groups();
        for w in groups.windows(2) {
            assert!(w[0] < w[1], "Groups not sorted: {:?} >= {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn known_groups_expected_set() {
        let groups = known_groups();
        let expected = vec![
            "Acquisition",
            "Corrections",
            "Device",
            "I/O",
            "Intensifier",
            "Metadata",
            "ROI",
            "Readout",
            "Sensor",
            "Timing",
        ];
        assert_eq!(groups, expected);
    }

    #[test]
    fn feature_type_display() {
        assert_eq!(format!("{}", FeatureType::Int), "int");
        assert_eq!(format!("{}", FeatureType::Float), "float");
        assert_eq!(format!("{}", FeatureType::Bool), "bool");
        assert_eq!(format!("{}", FeatureType::Enum), "enum");
        assert_eq!(format!("{}", FeatureType::Str), "string");
        assert_eq!(format!("{}", FeatureType::Command), "command");
    }

    #[test]
    fn mock_features_are_realistic() {
        let features = introspect_mock_features();
        assert!(
            features.len() >= 50,
            "Mock should return at least 50 features, got {}",
            features.len()
        );

        let names: Vec<&str> = features.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"ExposureTime"));
        assert!(names.contains(&"MCPGain"));
        assert!(names.contains(&"GateMode"));
        assert!(names.contains(&"SensorWidth"));
        assert!(names.contains(&"CameraModel"));
        assert!(names.contains(&"AcquisitionStart"));
    }

    #[test]
    fn mock_features_have_correct_types() {
        let features = introspect_mock_features();

        let exposure = features
            .iter()
            .find(|f| f.name == "ExposureTime")
            .expect("ExposureTime");
        assert_eq!(exposure.feature_type, FeatureType::Float);
        assert!(exposure.float_range.is_some());
        assert!(exposure.writable);

        let mcp = features
            .iter()
            .find(|f| f.name == "MCPGain")
            .expect("MCPGain");
        assert_eq!(mcp.feature_type, FeatureType::Int);
        assert_eq!(mcp.int_range, Some((0, 4095)));

        let trigger = features
            .iter()
            .find(|f| f.name == "TriggerMode")
            .expect("TriggerMode");
        assert_eq!(trigger.feature_type, FeatureType::Enum);
        assert!(!trigger.enum_values.is_empty());

        let model = features
            .iter()
            .find(|f| f.name == "CameraModel")
            .expect("CameraModel");
        assert_eq!(model.feature_type, FeatureType::Str);
        assert!(!model.writable);
    }

    #[test]
    fn mock_features_displayable_and_editable() {
        let features = introspect_mock_features();

        let exposure = features
            .iter()
            .find(|f| f.name == "ExposureTime")
            .expect("ExposureTime");
        assert!(exposure.is_displayable());
        assert!(exposure.is_editable());

        let sensor_w = features
            .iter()
            .find(|f| f.name == "SensorWidth")
            .expect("SensorWidth");
        assert!(sensor_w.is_displayable());
        assert!(!sensor_w.is_editable());

        let dev_count = features
            .iter()
            .find(|f| f.name == "DeviceCount")
            .expect("DeviceCount");
        assert!(!dev_count.is_displayable());
        assert!(!dev_count.is_editable());
    }

    #[test]
    fn mock_no_duplicate_names() {
        let features = introspect_mock_features();
        let mut names: Vec<&str> = features.iter().map(|f| f.name.as_str()).collect();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            original_len,
            "Mock has duplicate feature names"
        );
    }
}
