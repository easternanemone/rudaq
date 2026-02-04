//! Build script for andor-sdk3-sys FFI bindings.
//!
//! This script generates Rust FFI bindings from the Andor SDK3 C headers
//! using bindgen. It supports two modes:
//!
//! 1. With `andor-sdk3` feature: Generates bindings from SDK headers (Windows only)
//! 2. Without feature: Uses pre-generated dummy bindings for cross-compilation

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-env-changed=ANDOR_SDK3_DIR");

    #[cfg(feature = "andor-sdk3")]
    generate_bindings();

    #[cfg(not(feature = "andor-sdk3"))]
    generate_dummy_bindings();
}

#[cfg(feature = "andor-sdk3")]
fn generate_bindings() {
    // Get SDK directory from environment
    let sdk_dir = env::var("ANDOR_SDK3_DIR").expect(
        "ANDOR_SDK3_DIR environment variable must be set when `andor-sdk3` feature is enabled",
    );

    let sdk_path = PathBuf::from(&sdk_dir);
    let include_path = sdk_path.join("include");

    if !include_path.exists() {
        panic!("Andor SDK3 include path does not exist: {:?}", include_path);
    }

    println!(
        "cargo:rerun-if-changed={}",
        include_path.join("atcore.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        include_path.join("atspectrograph.h").display()
    );

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", include_path.display()))
        // Allow all AT functions (camera + spectrograph)
        .allowlist_function("AT_.*")
        .allowlist_function("ATSpectrograph.*")
        // Allow Andor types
        .allowlist_type("AT_.*")
        // Allow Andor constants
        .allowlist_var("AT_.*")
        // Use default enum style to keep constants at top level
        .default_enum_style(bindgen::EnumVariation::Consts)
        // Derive common traits
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        // Parse block comments as doc comments
        .generate_comments(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    // Add Windows-specific configuration for wide strings
    #[cfg(target_os = "windows")]
    {
        builder = builder
            .clang_arg("-DWINDOWS")
            .clang_arg("-D_UNICODE")
            .clang_arg("-DUNICODE");
    }

    let bindings = builder
        .generate()
        .expect("Unable to generate Andor SDK3 bindings");

    let out_path = PathBuf::from(
        env::var("OUT_DIR").expect("OUT_DIR environment variable must be set by Cargo"),
    );
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    // Link against Andor SDK3 libraries (Windows only)
    #[cfg(target_os = "windows")]
    {
        let lib_path = sdk_path.join("lib");
        println!("cargo:rustc-link-search=native={}", lib_path.display());

        // Link camera library
        #[cfg(feature = "camera")]
        println!("cargo:rustc-link-lib=atcore");

        // Link spectrograph library
        #[cfg(feature = "spectrograph")]
        println!("cargo:rustc-link-lib=atspectrograph");
    }
}

/// Generate dummy bindings when SDK is not available.
/// This allows the crate to compile on systems without Andor SDK3 installed.
#[cfg(not(feature = "andor-sdk3"))]
fn generate_dummy_bindings() {
    let out_path = PathBuf::from(
        env::var("OUT_DIR").expect("OUT_DIR environment variable must be set by Cargo"),
    );
    let dummy = r#"
// Dummy bindings - andor-sdk3 feature not enabled
//
// These are placeholder types and functions that allow the crate to compile
// without the actual Andor SDK3 headers. Enable the `andor-sdk3` feature
// to generate real bindings.

use std::os::raw::{c_char, c_int, c_double};

/// Andor handle type (opaque)
pub type AT_H = c_int;

/// Wide character type for Windows (UTF-16)
pub type AT_WC = u16;

/// Boolean type
pub type AT_BOOL = c_int;

/// 64-bit integer
pub type AT_64 = i64;

// Error codes
pub const AT_SUCCESS: c_int = 0;
pub const AT_ERR_NOTINITIALISED: c_int = 1;
pub const AT_ERR_NOTIMPLEMENTED: c_int = 2;
pub const AT_ERR_READONLY: c_int = 3;
pub const AT_ERR_NOTREADABLE: c_int = 4;
pub const AT_ERR_NOTWRITABLE: c_int = 5;
pub const AT_ERR_OUTOFRANGE: c_int = 6;
pub const AT_ERR_INDEXNOTAVAILABLE: c_int = 7;
pub const AT_ERR_INDEXNOTIMPLEMENTED: c_int = 8;
pub const AT_ERR_EXCEEDEDMAXSTRINGLENGTH: c_int = 9;
pub const AT_ERR_CONNECTION: c_int = 10;
pub const AT_ERR_NODATA: c_int = 11;
pub const AT_ERR_INVALIDHANDLE: c_int = 12;
pub const AT_ERR_TIMEDOUT: c_int = 13;
pub const AT_ERR_BUFFERFULL: c_int = 14;
pub const AT_ERR_INVALIDSIZE: c_int = 15;
pub const AT_ERR_INVALIDALIGNMENT: c_int = 16;
pub const AT_ERR_COMM: c_int = 17;
pub const AT_ERR_STRINGNOTAVAILABLE: c_int = 18;
pub const AT_ERR_STRINGNOTIMPLEMENTED: c_int = 19;
pub const AT_ERR_NULL_FEATURE: c_int = 20;
pub const AT_ERR_NULL_HANDLE: c_int = 21;
pub const AT_ERR_NULL_IMPLEMENTED_VAR: c_int = 22;
pub const AT_ERR_NULL_READABLE_VAR: c_int = 23;
pub const AT_ERR_NULL_READONLY_VAR: c_int = 24;
pub const AT_ERR_NULL_WRITABLE_VAR: c_int = 25;
pub const AT_ERR_NULL_MINVALUE: c_int = 26;
pub const AT_ERR_NULL_MAXVALUE: c_int = 27;
pub const AT_ERR_NULL_VALUE: c_int = 28;
pub const AT_ERR_NULL_STRING: c_int = 29;
pub const AT_ERR_NULL_COUNT_VAR: c_int = 30;
pub const AT_ERR_NULL_ISAVAILABLE_VAR: c_int = 31;
pub const AT_ERR_NULL_MAXSTRINGLENGTH: c_int = 32;
pub const AT_ERR_NULL_EVCALLBACK: c_int = 33;
pub const AT_ERR_NULL_QUEUE_PTR: c_int = 34;
pub const AT_ERR_NULL_WAIT_PTR: c_int = 35;
pub const AT_ERR_NULL_PTRSIZE: c_int = 36;
pub const AT_ERR_NOMEMORY: c_int = 37;
pub const AT_ERR_DEVICEINUSE: c_int = 38;
pub const AT_ERR_HARDWARE_OVERFLOW: c_int = 100;

// Handle constants
pub const AT_HANDLE_UNINITIALISED: AT_H = -1;
pub const AT_HANDLE_SYSTEM: AT_H = 1;

// Boolean constants
pub const AT_TRUE: AT_BOOL = 1;
pub const AT_FALSE: AT_BOOL = 0;

// Callback infinite wait
pub const AT_CALLBACK_SUCCESS: c_int = 0;

// Panic stub implementations for camera functions
const ANDOR_SDK3_PANIC_MSG: &str = "Andor SDK3 function called but andor-sdk3 feature is not enabled. \
    Enable the andor-sdk3 feature (or hardware in driver-andor) to use the real Andor SDK3.";

// Camera library functions (atcore.dll)
#[no_mangle]
pub unsafe extern "C" fn AT_InitialiseLibrary() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_FinaliseLibrary() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_Open(_device_index: c_int, _handle: *mut AT_H) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_Close(_handle: AT_H) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_SetInt(_handle: AT_H, _feature: *const AT_WC, _value: AT_64) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetInt(_handle: AT_H, _feature: *const AT_WC, _value: *mut AT_64) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_SetFloat(_handle: AT_H, _feature: *const AT_WC, _value: c_double) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetFloat(_handle: AT_H, _feature: *const AT_WC, _value: *mut c_double) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_SetEnumString(_handle: AT_H, _feature: *const AT_WC, _string: *const AT_WC) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetEnumStringByIndex(
    _handle: AT_H,
    _feature: *const AT_WC,
    _index: c_int,
    _string: *mut AT_WC,
    _string_length: c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_Command(_handle: AT_H, _feature: *const AT_WC) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_QueueBuffer(_handle: AT_H, _ptr: *mut u8, _ptr_size: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_WaitBuffer(
    _handle: AT_H,
    _ptr: *mut *mut u8,
    _ptr_size: *mut c_int,
    _timeout: c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_Flush(_handle: AT_H) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetString(
    _handle: AT_H,
    _feature: *const AT_WC,
    _string: *mut AT_WC,
    _string_length: c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_SetBool(_handle: AT_H, _feature: *const AT_WC, _value: AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// Spectrograph library constants
pub const SHAMROCK_SUCCESS: c_int = 20202;  // Shamrock success code

// Spectrograph library functions (atspectrograph.dll / Shamrock API)
#[no_mangle]
pub unsafe extern "C" fn ShamrockInitialize(_path: *mut c_char) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockClose() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetNumberDevices(_num_devices: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetSerialNumber(_device: c_int, _serial: *mut c_char) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetGrating(_device: c_int, _grating: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetGrating(_device: c_int, _grating: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetNumberGratings(_device: c_int, _num_gratings: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetGratingInfo(
    _device: c_int,
    _grating: c_int,
    _lines: *mut c_double,
    _blaze: *mut c_double,
    _home: *mut c_int,
    _offset: *mut c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetWavelength(_device: c_int, _wavelength: f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetWavelength(_device: c_int, _wavelength: *mut f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetAutoSlitWidth(_device: c_int, _slit: c_int, _width: f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetAutoSlitWidth(_device: c_int, _slit: c_int, _width: *mut f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetCalibration(
    _device: c_int,
    _calibration: *mut f32,
    _num_pixels: c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetShutter(_device: c_int, _mode: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetFlipperMirror(_device: c_int, _port: c_int, _pos: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}
"#;

    std::fs::write(out_path.join("bindings.rs"), dummy).expect("Couldn't write dummy bindings!");
}
