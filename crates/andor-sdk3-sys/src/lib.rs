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
    fn test_constants() {
        // Verify error codes are defined
        assert_eq!(AT_SUCCESS, 0);
        assert_eq!(AT_HANDLE_UNINITIALISED, -1);
        assert_eq!(AT_HANDLE_SYSTEM, 1);
    }

    #[test]
    fn test_boolean_constants() {
        assert_eq!(AT_TRUE, 1);
        assert_eq!(AT_FALSE, 0);
    }
}
