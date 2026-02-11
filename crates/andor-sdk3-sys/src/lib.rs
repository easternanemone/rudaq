//! Low-level FFI bindings for the Andor SDK3.
//!
//! This crate provides raw, unsafe bindings to the Andor SDK3 C library,
//! which supports both Andor cameras and spectrographs.
//!
//! # Andor SDK3 Overview
//!
//! Andor SDK3 is a modern C API for controlling Andor scientific cameras
//! (Neo, Zyla, Marana) and spectrographs (Shamrock series). It uses a
//! property-based interface with wide strings (UTF-16 on Windows).
//!
//! The SDK consists of two main components:
//! - `atcore.dll` - Camera control (frame acquisition, settings)
//! - `atspectrograph.dll` - Spectrograph control (grating, wavelength)
//!
//! # Safety
//!
//! All functions in this crate are `unsafe` as they are direct FFI bindings.
//! For a safe wrapper, use the `driver-andor` crate instead.
//!
//! # Features
//!
//! - `andor-sdk3`: Generate bindings from SDK headers (Windows only).
//!   Without this feature, pre-defined bindings are used for cross-compilation.
//! - `camera`: Enable camera API functions
//! - `spectrograph`: Enable spectrograph API functions
//! - `hardware`: Enable all features for real hardware
//!
//! # Wide String Handling
//!
//! Andor SDK3 uses wide strings (UTF-16) for feature names and enum values.
//! This crate provides helper functions for converting between Rust strings
//! and wide strings.
//!
//! # SimCam (Simulated Camera) — Testing Without Hardware
//!
//! The Andor SDK3 includes a **SimCam** virtual device that emulates a real
//! camera on the host machine. This is invaluable for development and CI
//! testing where no physical Andor camera is available.
//!
//! ## How SimCam Works
//!
//! When the SDK is installed on a system, the SimCam driver is registered
//! automatically. It appears as an additional device alongside any real
//! cameras. Key characteristics:
//!
//! - **Device index**: SimCam is typically the *last* device index.
//!   With zero physical cameras, it is device index 0.
//! - **System handle**: `AT_HANDLE_SYSTEM` (index 1) always exists and can
//!   be used to query `DeviceCount` to discover the SimCam.
//! - **Feature support**: SimCam supports most features including
//!   `SensorWidth`, `SensorHeight`, `ExposureTime`, `PixelEncoding`,
//!   trigger modes, and frame acquisition via `AT_QueueBuffer`/`AT_WaitBuffer`.
//! - **Simulated frames**: The SimCam generates synthetic image data
//!   (gradient patterns) that can be used to verify acquisition pipelines.
//!
//! ## Using SimCam
//!
//! On **Windows** with the SDK installed:
//!
//! 1. Initialize the library: `AT_InitialiseLibrary()`
//! 2. Query device count via system handle: `AT_GetInt(AT_HANDLE_SYSTEM, "DeviceCount", &count)`
//! 3. Open the SimCam (usually the last device): `AT_Open(count - 1, &handle)`
//! 4. Use the handle like any real camera
//!
//! On **non-Windows** or without the SDK (cross-compilation mode):
//!
//! The dummy bindings in this crate panic on any SDK call. For testing
//! without the SDK, use the mock backend provided by `driver-andor-sdk3`
//! (enabled when the `spectrograph`/`camera` features are disabled):
//!
//! ```rust,ignore
//! use driver_andor_sdk3::camera::AndorCamera;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // This uses the mock backend — no SDK needed
//! let camera = AndorCamera::new_mock().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## SimCam Limitations
//!
//! - No real photon data — frames contain synthetic patterns only
//! - Temperature features report fixed values
//! - iStar-specific features (MCP, DDG, gate) may not be available
//! - Timing is simulated and may not match real hardware latency
//!
//! ## Detecting SimCam at Runtime
//!
//! The SimCam identifies itself as `"SIMCAM CMOS"` in the `CameraModel`
//! feature. Check this to distinguish it from real hardware:
//!
//! ```no_run
//! use andor_sdk3_sys::*;
//!
//! # unsafe {
//! # let handle: AT_H = 0;
//! let feature = to_wide_string("CameraModel");
//! let mut buffer = wide_string_buffer(256);
//! AT_GetString(handle, feature.as_ptr(), buffer.as_mut_ptr(), 256);
//! let model = from_wide_string(&buffer);
//! let is_simcam = model.contains("SIMCAM");
//! # }
//! ```
//!
//! # Example (unsafe)
//!
//! ```no_run
//! use andor_sdk3_sys::*;
//!
//! unsafe {
//!     // Initialize library
//!     let ret = AT_InitialiseLibrary();
//!     assert_eq!(ret, AT_SUCCESS);
//!
//!     // Open camera
//!     let mut handle = AT_HANDLE_UNINITIALISED;
//!     let ret = AT_Open(0, &mut handle);
//!     assert_eq!(ret, AT_SUCCESS);
//!
//!     // Get sensor width (using wide string)
//!     let feature = to_wide_string("SensorWidth");
//!     let mut width: AT_64 = 0;
//!     let ret = AT_GetInt(handle, feature.as_ptr(), &mut width);
//!     assert_eq!(ret, AT_SUCCESS);
//!
//!     // Close camera
//!     AT_Close(handle);
//!     AT_FinaliseLibrary();
//! }
//! ```

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]
#![allow(unsafe_code)] // FFI bindings require unsafe

use widestring::U16CString;

// Include the generated bindings
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Wide string helper functions for Andor SDK3
// The SDK uses UTF-16 strings (AT_WC = wchar_t on Windows)

/// Convert a Rust string to a wide string (UTF-16) for Andor SDK3.
///
/// # Panics
///
/// Panics if the input string contains interior null bytes (U+0000).
/// This should never happen with valid SDK feature names.
///
/// # Example
///
/// ```no_run
/// use andor_sdk3_sys::{to_wide_string, AT_GetInt, AT_64};
///
/// unsafe {
///     let handle = 1; // Assume valid handle
///     let feature = to_wide_string("SensorWidth");
///     let mut value: AT_64 = 0;
///     AT_GetInt(handle, feature.as_ptr(), &mut value);
/// }
/// ```
pub fn to_wide_string(s: &str) -> U16CString {
    U16CString::from_str(s).expect("Failed to convert string to wide string")
}

/// Convert a wide string buffer to a Rust String.
///
/// This is useful for reading string values from the Andor SDK3.
///
/// # Safety
///
/// The buffer must be a valid null-terminated UTF-16 string.
///
/// # Example
///
/// ```no_run
/// use andor_sdk3_sys::{from_wide_string, to_wide_string, AT_GetEnumStringByIndex};
///
/// unsafe {
///     let handle = 1; // Assume valid handle
///     let feature = to_wide_string("PixelEncoding");
///     let mut buffer = vec![0u16; 256];
///     AT_GetEnumStringByIndex(handle, feature.as_ptr(), 0, buffer.as_mut_ptr(), 256);
///     let value = from_wide_string(&buffer);
/// }
/// ```
pub fn from_wide_string(buffer: &[u16]) -> String {
    // Find the null terminator
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Create a wide string buffer of specified size.
///
/// This is useful for receiving string values from the Andor SDK3.
pub fn wide_string_buffer(size: usize) -> Vec<u16> {
    vec![0u16; size]
}

/// Andor SDK3 feature name constants.
///
/// These are the wide-string feature names used with `AT_GetInt`, `AT_SetFloat`,
/// `AT_SetEnumString`, etc. Using these constants avoids typos and enables
/// compile-time checking of feature names.
///
/// Reference: Andor SDK3 Software Development Kit manual, Feature Reference.
pub mod features {
    // ── Sensor / image geometry ─────────────────────────────────────────
    pub const SENSOR_WIDTH: &str = "SensorWidth";
    pub const SENSOR_HEIGHT: &str = "SensorHeight";
    pub const AOI_WIDTH: &str = "AOIWidth";
    pub const AOI_HEIGHT: &str = "AOIHeight";
    pub const AOI_LEFT: &str = "AOILeft";
    pub const AOI_TOP: &str = "AOITop";
    pub const AOI_STRIDE: &str = "AOIStride";
    pub const AOI_HBIN: &str = "AOIHBin";
    pub const AOI_VBIN: &str = "AOIVBin";
    pub const IMAGE_SIZE_BYTES: &str = "ImageSizeBytes";

    // ── Acquisition control ─────────────────────────────────────────────
    pub const ACQUISITION_START: &str = "AcquisitionStart";
    pub const ACQUISITION_STOP: &str = "AcquisitionStop";
    pub const CYCLE_MODE: &str = "CycleMode";
    pub const FRAME_COUNT: &str = "FrameCount";
    pub const FRAME_RATE: &str = "FrameRate";
    pub const MAX_INTERFACE_TRANSFER_RATE: &str = "MaxInterfaceTransferRate";
    pub const ACCUMULATE_COUNT: &str = "AccumulateCount";

    // ── Exposure ────────────────────────────────────────────────────────
    pub const EXPOSURE_TIME: &str = "ExposureTime";
    pub const READOUT_TIME: &str = "ReadoutTime";

    // ── Trigger ─────────────────────────────────────────────────────────
    pub const TRIGGER_MODE: &str = "TriggerMode";
    pub const SOFTWARE_TRIGGER: &str = "SoftwareTrigger";
    pub const ELECTRONIC_SHUTTERING_MODE: &str = "ElectronicShutteringMode";

    // ── Pixel encoding ──────────────────────────────────────────────────
    pub const PIXEL_ENCODING: &str = "PixelEncoding";
    pub const PIXEL_READOUT_RATE: &str = "PixelReadoutRate";
    pub const PIXEL_WIDTH: &str = "PixelWidth";
    pub const PIXEL_HEIGHT: &str = "PixelHeight";
    pub const BYTES_PER_PIXEL: &str = "BytesPerPixel";
    pub const SIMPLE_PRE_AMP_GAIN_CONTROL: &str = "SimplePreAmpGainControl";

    // ── Temperature ─────────────────────────────────────────────────────
    pub const SENSOR_TEMPERATURE: &str = "SensorTemperature";
    pub const TEMPERATURE_STATUS: &str = "TemperatureStatus";
    pub const TEMPERATURE_CONTROL: &str = "TemperatureControl";
    pub const TARGET_SENSOR_TEMPERATURE: &str = "TargetSensorTemperature";
    pub const FAN_SPEED: &str = "FanSpeed";
    pub const SENSOR_COOLING: &str = "SensorCooling";

    // ── Device identity ─────────────────────────────────────────────────
    pub const CAMERA_MODEL: &str = "CameraModel";
    pub const SERIAL_NUMBER: &str = "SerialNumber";
    pub const CAMERA_NAME: &str = "CameraName";
    pub const FIRMWARE_VERSION: &str = "FirmwareVersion";
    pub const INTERFACE_TYPE: &str = "InterfaceType";
    pub const CAMERA_FAMILY: &str = "CameraFamily";
    pub const CONTROLLER_ID: &str = "ControllerID";
    pub const DEVICE_COUNT: &str = "DeviceCount";

    // ── Buffer / timestamping ───────────────────────────────────────────
    pub const BUFFER_OVERFLOW_EVENT: &str = "BufferOverflowEvent";
    pub const EVENT_ENABLE: &str = "EventEnable";
    pub const EVENTS_MISSED_EVENT: &str = "EventsMissedEvent";
    pub const TIMESTAMP_CLOCK: &str = "TimestampClock";
    pub const TIMESTAMP_CLOCK_FREQUENCY: &str = "TimestampClockFrequency";
    pub const TIMESTAMP_CLOCK_RESET: &str = "TimestampClockReset";
    pub const METADATA_ENABLE: &str = "MetadataEnable";
    pub const METADATA_FRAME: &str = "MetadataFrame";
    pub const METADATA_TIMESTAMP: &str = "MetadataTimestamp";

    // ── iStar / intensifier features (DDG, MCP, gate) ───────────────────
    pub const GATE_MODE: &str = "GateMode";
    pub const MCP_GAIN: &str = "MCPGain";
    pub const MCP_INTELLIGENT_GATING: &str = "MCPIntelligentGating";
    pub const INSERTION_DELAY: &str = "InsertionDelay";
    pub const DDG_OUTPUT_DELAY: &str = "DDGOutputDelay";
    pub const DDG_OUTPUT_WIDTH: &str = "DDGOutputWidth";
    pub const DDG_STEP_ENABLED: &str = "DDGStepEnabled";
    pub const DDG_STEP_COUNT: &str = "DDGStepCount";
    pub const DDG_STEP_DELAY_COEFFICIENT: &str = "DDGStepDelayCoefficient";
    pub const DDG_STEP_WIDTH_COEFFICIENT: &str = "DDGStepWidthCoefficient";
    pub const DDG_STEP_UPLOAD_PROGRESS: &str = "DDGStepUploadProgress";
    pub const DDG_STEP_UPLOAD_REQUIRED: &str = "DDGStepUploadRequired";

    // ── I/O control ─────────────────────────────────────────────────────
    pub const IO_INVERT: &str = "IOInvert";
    pub const IO_SELECTOR: &str = "IOSelector";
    pub const AUXILIARY_OUT_SOURCE: &str = "AuxiliaryOutSource";
    pub const AUX_OUT_SOURCE_TWO: &str = "AuxOutSourceTwo";

    // ── Miscellaneous ───────────────────────────────────────────────────
    pub const SPURIOUS_NOISE_FILTER: &str = "SpuriousNoiseFilter";
    pub const STATIC_BLEMISH_CORRECTION: &str = "StaticBlemishCorrection";
    pub const OVERLAP: &str = "Overlap";
    pub const ROLLING_SHUTTER_GLOBAL_CLEAR: &str = "RollingShutterGlobalClear";
    pub const VERTICALLY_CENTRE_AOI: &str = "VerticallyCentreAOI";
    pub const FULL_AOI_CONTROL: &str = "FullAOIControl";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wide_string_conversion() {
        let s = "SensorWidth";
        let wide = to_wide_string(s);
        let wide_slice: Vec<u16> = wide.as_slice().to_vec();

        // Convert back
        let back = from_wide_string(&wide_slice);
        assert_eq!(s, back);
    }

    #[test]
    fn test_wide_string_buffer() {
        let buffer = wide_string_buffer(256);
        assert_eq!(buffer.len(), 256);
        assert_eq!(buffer[0], 0);
    }

    #[test]
    fn test_handle_constants() {
        assert_eq!(AT_SUCCESS, 0);
        assert_eq!(AT_HANDLE_UNINITIALISED, -1);
        assert_eq!(AT_HANDLE_SYSTEM, 1);
    }

    #[test]
    fn test_boolean_constants() {
        assert_eq!(AT_TRUE, 1);
        assert_eq!(AT_FALSE, 0);
    }

    // ── AT_ERR_* error code constant tests [bd-p1xp] ───────────────────

    #[test]
    fn test_error_codes_general() {
        assert_eq!(AT_ERR_NOTINITIALISED, 1);
        assert_eq!(AT_ERR_NOTIMPLEMENTED, 2);
        assert_eq!(AT_ERR_READONLY, 3);
        assert_eq!(AT_ERR_NOTREADABLE, 4);
        assert_eq!(AT_ERR_NOTWRITABLE, 5);
        assert_eq!(AT_ERR_OUTOFRANGE, 6);
    }

    #[test]
    fn test_error_codes_index() {
        assert_eq!(AT_ERR_INDEXNOTAVAILABLE, 7);
        assert_eq!(AT_ERR_INDEXNOTIMPLEMENTED, 8);
    }

    #[test]
    fn test_error_codes_connection() {
        assert_eq!(AT_ERR_EXCEEDEDMAXSTRINGLENGTH, 9);
        assert_eq!(AT_ERR_CONNECTION, 10);
        assert_eq!(AT_ERR_NODATA, 11);
        assert_eq!(AT_ERR_INVALIDHANDLE, 12);
        assert_eq!(AT_ERR_TIMEDOUT, 13);
        assert_eq!(AT_ERR_BUFFERFULL, 14);
        assert_eq!(AT_ERR_INVALIDSIZE, 15);
        assert_eq!(AT_ERR_INVALIDALIGNMENT, 16);
        assert_eq!(AT_ERR_COMM, 17);
    }

    #[test]
    fn test_error_codes_string() {
        assert_eq!(AT_ERR_STRINGNOTAVAILABLE, 18);
        assert_eq!(AT_ERR_STRINGNOTIMPLEMENTED, 19);
    }

    #[test]
    fn test_error_codes_null_parameter() {
        assert_eq!(AT_ERR_NULL_FEATURE, 20);
        assert_eq!(AT_ERR_NULL_HANDLE, 21);
        assert_eq!(AT_ERR_NULL_IMPLEMENTED_VAR, 22);
        assert_eq!(AT_ERR_NULL_READABLE_VAR, 23);
        assert_eq!(AT_ERR_NULL_READONLY_VAR, 24);
        assert_eq!(AT_ERR_NULL_WRITABLE_VAR, 25);
        assert_eq!(AT_ERR_NULL_MINVALUE, 26);
        assert_eq!(AT_ERR_NULL_MAXVALUE, 27);
        assert_eq!(AT_ERR_NULL_VALUE, 28);
        assert_eq!(AT_ERR_NULL_STRING, 29);
        assert_eq!(AT_ERR_NULL_COUNT_VAR, 30);
        assert_eq!(AT_ERR_NULL_ISAVAILABLE_VAR, 31);
        assert_eq!(AT_ERR_NULL_MAXSTRINGLENGTH, 32);
        assert_eq!(AT_ERR_NULL_EVCALLBACK, 33);
        assert_eq!(AT_ERR_NULL_QUEUE_PTR, 34);
        assert_eq!(AT_ERR_NULL_WAIT_PTR, 35);
        assert_eq!(AT_ERR_NULL_PTRSIZE, 36);
    }

    #[test]
    fn test_error_codes_device() {
        assert_eq!(AT_ERR_NOMEMORY, 37);
        assert_eq!(AT_ERR_DEVICEINUSE, 38);
        assert_eq!(AT_ERR_DEVICENOTFOUND, 39);
    }

    #[test]
    fn test_error_codes_hardware() {
        assert_eq!(AT_ERR_HARDWARE_OVERFLOW, 100);
    }

    #[test]
    fn test_error_codes_are_contiguous_0_to_39() {
        // Verify that error codes 0-39 form a contiguous range
        // (no gaps from AT_SUCCESS to AT_ERR_DEVICENOTFOUND)
        let expected: Vec<i32> = (0..=39).collect();
        let actual = vec![
            AT_SUCCESS,
            AT_ERR_NOTINITIALISED,
            AT_ERR_NOTIMPLEMENTED,
            AT_ERR_READONLY,
            AT_ERR_NOTREADABLE,
            AT_ERR_NOTWRITABLE,
            AT_ERR_OUTOFRANGE,
            AT_ERR_INDEXNOTAVAILABLE,
            AT_ERR_INDEXNOTIMPLEMENTED,
            AT_ERR_EXCEEDEDMAXSTRINGLENGTH,
            AT_ERR_CONNECTION,
            AT_ERR_NODATA,
            AT_ERR_INVALIDHANDLE,
            AT_ERR_TIMEDOUT,
            AT_ERR_BUFFERFULL,
            AT_ERR_INVALIDSIZE,
            AT_ERR_INVALIDALIGNMENT,
            AT_ERR_COMM,
            AT_ERR_STRINGNOTAVAILABLE,
            AT_ERR_STRINGNOTIMPLEMENTED,
            AT_ERR_NULL_FEATURE,
            AT_ERR_NULL_HANDLE,
            AT_ERR_NULL_IMPLEMENTED_VAR,
            AT_ERR_NULL_READABLE_VAR,
            AT_ERR_NULL_READONLY_VAR,
            AT_ERR_NULL_WRITABLE_VAR,
            AT_ERR_NULL_MINVALUE,
            AT_ERR_NULL_MAXVALUE,
            AT_ERR_NULL_VALUE,
            AT_ERR_NULL_STRING,
            AT_ERR_NULL_COUNT_VAR,
            AT_ERR_NULL_ISAVAILABLE_VAR,
            AT_ERR_NULL_MAXSTRINGLENGTH,
            AT_ERR_NULL_EVCALLBACK,
            AT_ERR_NULL_QUEUE_PTR,
            AT_ERR_NULL_WAIT_PTR,
            AT_ERR_NULL_PTRSIZE,
            AT_ERR_NOMEMORY,
            AT_ERR_DEVICEINUSE,
            AT_ERR_DEVICENOTFOUND,
        ];
        assert_eq!(actual, expected);
    }

    // ── Shamrock error / port constant tests ────────────────────────────

    #[test]
    fn test_shamrock_constants() {
        assert_eq!(SHAMROCK_SUCCESS, 20202);
        assert_eq!(SHAMROCK_COMMUNICATION_ERROR, 20201);
        assert_eq!(SHAMROCK_NOT_INITIALIZED, 20275);
        assert_eq!(SHAMROCK_NOT_AVAILABLE, 20292);
    }

    #[test]
    fn test_shamrock_parameter_error_codes() {
        assert_eq!(SHAMROCK_P1INVALID, 20266);
        assert_eq!(SHAMROCK_P2INVALID, 20267);
        assert_eq!(SHAMROCK_P3INVALID, 20268);
        assert_eq!(SHAMROCK_P4INVALID, 20269);
    }

    #[test]
    fn test_shamrock_slit_port_constants() {
        assert_eq!(SHAMROCK_INPUT_SLIT_SIDE, 1);
        assert_eq!(SHAMROCK_INPUT_SLIT_DIRECT, 2);
        assert_eq!(SHAMROCK_OUTPUT_SLIT_SIDE, 3);
        assert_eq!(SHAMROCK_OUTPUT_SLIT_DIRECT, 4);
    }

    #[test]
    fn test_shamrock_flipper_port_constants() {
        assert_eq!(SHAMROCK_FLIPPER_INPUT, 1);
        assert_eq!(SHAMROCK_FLIPPER_OUTPUT, 2);
    }

    // ── Feature name constants tests [bd-t2os] ──────────────────────────

    #[test]
    fn test_feature_names_sensor() {
        assert_eq!(features::SENSOR_WIDTH, "SensorWidth");
        assert_eq!(features::SENSOR_HEIGHT, "SensorHeight");
        assert_eq!(features::AOI_WIDTH, "AOIWidth");
        assert_eq!(features::AOI_HEIGHT, "AOIHeight");
    }

    #[test]
    fn test_feature_names_acquisition() {
        assert_eq!(features::ACQUISITION_START, "AcquisitionStart");
        assert_eq!(features::ACQUISITION_STOP, "AcquisitionStop");
        assert_eq!(features::EXPOSURE_TIME, "ExposureTime");
        assert_eq!(features::TRIGGER_MODE, "TriggerMode");
    }

    #[test]
    fn test_feature_names_device_identity() {
        assert_eq!(features::CAMERA_MODEL, "CameraModel");
        assert_eq!(features::SERIAL_NUMBER, "SerialNumber");
        assert_eq!(features::FIRMWARE_VERSION, "FirmwareVersion");
    }

    #[test]
    fn test_feature_names_can_convert_to_wide_string() {
        // Verify all feature name constants can be converted to wide strings
        let names = [
            features::SENSOR_WIDTH,
            features::SENSOR_HEIGHT,
            features::EXPOSURE_TIME,
            features::TRIGGER_MODE,
            features::PIXEL_ENCODING,
            features::CAMERA_MODEL,
            features::SERIAL_NUMBER,
            features::SENSOR_TEMPERATURE,
            features::GATE_MODE,
            features::MCP_GAIN,
        ];
        for name in &names {
            let wide = to_wide_string(name);
            let back = from_wide_string(wide.as_slice());
            assert_eq!(*name, back, "Round-trip failed for feature: {}", name);
        }
    }
}
