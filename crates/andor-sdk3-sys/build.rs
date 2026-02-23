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
        // atcore.h uses wchar_t without including <wchar.h>
        .clang_arg("-include")
        .clang_arg("wchar.h");

    // Define feature macros so wrapper.h includes the correct headers
    #[cfg(feature = "camera")]
    {
        builder = builder.clang_arg("-DANDOR_SDK3_CAMERA");
    }

    // Only define spectrograph macro if the header actually exists
    #[cfg(feature = "spectrograph")]
    {
        if include_path.join("atspectrograph.h").exists() {
            builder = builder.clang_arg("-DANDOR_SDK3_SPECTROGRAPH");
        } else {
            println!(
                "cargo:warning=atspectrograph.h not found in {:?}, skipping spectrograph bindings",
                include_path
            );
        }
    }

    builder = builder
        // Allow all AT functions (camera + spectrograph)
        .allowlist_function("AT_.*")
        .allowlist_function("ATSpectrograph.*")
        .allowlist_function("Shamrock.*")
        // Allow Andor types
        .allowlist_type("AT_.*")
        // Allow Andor constants
        .allowlist_var("AT_.*")
        .allowlist_var("SHAMROCK_.*")
        // Use signed types for #define constants (matches function return types)
        .default_macro_constant_type(bindgen::MacroTypeVariation::Signed)
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
    let bindings_path = out_path.join("bindings.rs");
    bindings
        .write_to_file(&bindings_path)
        .expect("Couldn't write bindings!");

    // If spectrograph feature is enabled but header is missing, append stub bindings
    // so the spectrograph module compiles (stubs panic at runtime, same as dummy mode)
    #[cfg(feature = "spectrograph")]
    if !include_path.join("atspectrograph.h").exists() {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&bindings_path)
            .expect("Couldn't open bindings for appending");
        write!(f, "{}", SHAMROCK_STUBS).expect("Couldn't append Shamrock stubs");
    }

    // Link against Andor SDK3 libraries
    #[cfg(target_os = "windows")]
    {
        let lib_path = sdk_path.join("lib");
        println!("cargo:rustc-link-search=native={}", lib_path.display());
    }

    // On Linux, the SDK installs to /usr/local/lib (or ANDOR_SDK3_DIR/lib)
    #[cfg(target_os = "linux")]
    {
        let lib_path = sdk_path.join("lib");
        if lib_path.exists() {
            println!("cargo:rustc-link-search=native={}", lib_path.display());
        }
    }

    // Link camera library (atcore)
    #[cfg(feature = "camera")]
    println!("cargo:rustc-link-lib=atcore");

    // Link spectrograph library (atspectrograph) if header exists
    #[cfg(feature = "spectrograph")]
    if include_path.join("atspectrograph.h").exists() {
        println!("cargo:rustc-link-lib=atspectrograph");
    }
}

/// Shamrock spectrograph stub bindings for when the header is not available.
/// These allow the spectrograph module to compile but panic at runtime.
#[cfg(feature = "andor-sdk3")]
const SHAMROCK_STUBS: &str = r#"

// ══════════════════════════════════════════════════════════════════════════
// Shamrock spectrograph stubs (atspectrograph.h not found)
// ══════════════════════════════════════════════════════════════════════════

use std::os::raw::{c_char, c_int, c_double};

pub const SHAMROCK_SUCCESS: c_int = 20202;
pub const SHAMROCK_COMMUNICATION_ERROR: c_int = 20201;
pub const SHAMROCK_P1INVALID: c_int = 20266;
pub const SHAMROCK_P2INVALID: c_int = 20267;
pub const SHAMROCK_P3INVALID: c_int = 20268;
pub const SHAMROCK_P4INVALID: c_int = 20269;
pub const SHAMROCK_NOT_INITIALIZED: c_int = 20275;
pub const SHAMROCK_NOT_AVAILABLE: c_int = 20292;

pub const SHAMROCK_INPUT_SLIT_SIDE: c_int = 1;
pub const SHAMROCK_INPUT_SLIT_DIRECT: c_int = 2;
pub const SHAMROCK_OUTPUT_SLIT_SIDE: c_int = 3;
pub const SHAMROCK_OUTPUT_SLIT_DIRECT: c_int = 4;
pub const SHAMROCK_FLIPPER_INPUT: c_int = 1;
pub const SHAMROCK_FLIPPER_OUTPUT: c_int = 2;

// Stubs return SHAMROCK_NOT_AVAILABLE instead of panicking to avoid
// undefined behavior from unwinding across the extern "C" FFI boundary.

#[no_mangle] pub unsafe extern "C" fn ShamrockInitialize(_p: *mut c_char) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockClose() -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetNumberDevices(_n: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetSerialNumber(_d: c_int, _s: *mut c_char) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetGrating(_d: c_int, _g: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetGrating(_d: c_int, _g: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetNumberGratings(_d: c_int, _n: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetGratingInfo(_d: c_int, _g: c_int, _l: *mut c_double, _b: *mut c_double, _h: *mut c_int, _o: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGratingIsPresent(_d: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetGratingOffset(_d: c_int, _g: c_int, _o: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetGratingOffset(_d: c_int, _g: c_int, _o: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetWavelength(_d: c_int, _w: f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetWavelength(_d: c_int, _w: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetWavelengthLimits(_d: c_int, _g: c_int, _min: *mut f32, _max: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockWavelengthReset(_d: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockAutoSlitIsPresent(_d: c_int, _s: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetAutoSlitWidth(_d: c_int, _s: c_int, _w: f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetAutoSlitWidth(_d: c_int, _s: c_int, _w: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockAutoSlitReset(_d: c_int, _s: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSlitIsPresent(_d: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetSlit(_d: c_int, _w: f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetSlit(_d: c_int, _w: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetShutter(_d: c_int, _m: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetShutter(_d: c_int, _m: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockShutterIsPresent(_d: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetFlipperMirror(_d: c_int, _p: c_int, _pos: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetFlipperMirror(_d: c_int, _p: c_int, _pos: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockFlipperMirrorIsPresent(_d: c_int, _p: c_int, _pr: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockFlipperMirrorReset(_d: c_int, _p: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetCalibration(_d: c_int, _c: *mut f32, _n: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetPixelCalibrationCoefficients(_d: c_int, _a: *mut f32, _b: *mut f32, _c: *mut f32, _dd: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetPixelWidth(_d: c_int, _w: f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetPixelWidth(_d: c_int, _w: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetNumberPixels(_d: c_int, _n: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetNumberPixels(_d: c_int, _n: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetDetectorOffset(_d: c_int, _o: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetDetectorOffset(_d: c_int, _o: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockEepromGetOpticalParams(_d: c_int, _f: *mut f32, _a: *mut f32, _t: *mut f32) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockFocusMirrorIsPresent(_d: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetFocusMirror(_d: c_int, _f: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetFocusMirror(_d: c_int, _f: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetFocusMirrorMaxSteps(_d: c_int, _s: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockFilterIsPresent(_d: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetFilter(_d: c_int, _f: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetFilter(_d: c_int, _f: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetFilterInfo(_d: c_int, _f: c_int, _i: *mut c_char) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockAccessoryIsPresent(_d: c_int, _p: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockGetAccessoryState(_d: c_int, _a: c_int, _s: *mut c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
#[no_mangle] pub unsafe extern "C" fn ShamrockSetAccessory(_d: c_int, _a: c_int, _s: c_int) -> c_int { SHAMROCK_NOT_AVAILABLE }
"#;

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

use std::os::raw::{c_char, c_int, c_uint, c_double, c_void};

/// Andor handle type (opaque)
pub type AT_H = c_int;

/// Wide character type for Windows (UTF-16)
pub type AT_WC = u16;

/// Boolean type
pub type AT_BOOL = c_int;

/// 64-bit integer
pub type AT_64 = i64;

/// Unsigned 8-bit type (used in buffer conversion functions)
pub type AT_U8 = u8;

/// Feature callback function pointer type
///
/// Called when a feature value changes. Registered via AT_RegisterFeatureCallback.
///   handle:  camera handle the callback is registered on
///   feature: wide-string name of the feature that changed
///   context: user-supplied context pointer
pub type FeatureCallback = Option<
    unsafe extern "C" fn(handle: AT_H, feature: *const AT_WC, context: *mut c_void) -> c_int,
>;

// ── Error codes ──────────────────────────────────────────────────────────
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
pub const AT_ERR_DEVICENOTFOUND: c_int = 39;
pub const AT_ERR_HARDWARE_OVERFLOW: c_int = 100;

// ── Handle constants ─────────────────────────────────────────────────────
pub const AT_HANDLE_UNINITIALISED: AT_H = -1;
pub const AT_HANDLE_SYSTEM: AT_H = 1;

// ── Boolean constants ────────────────────────────────────────────────────
pub const AT_TRUE: AT_BOOL = 1;
pub const AT_FALSE: AT_BOOL = 0;

// ── Timeout / callback constants ─────────────────────────────────────────
pub const AT_INFINITE: c_int = 0xFFFFFFFF_u32 as c_int;
pub const AT_CALLBACK_SUCCESS: c_int = 0;

// ── Panic stub implementations ──────────────────────────────────────────
//
// When the `andor-sdk3` feature is *not* enabled the crate still compiles
// and links, but every SDK function panics at runtime.  This lets
// downstream crates compile unconditionally while keeping the real SDK
// optional.

const ANDOR_SDK3_PANIC_MSG: &str = "Andor SDK3 function called but andor-sdk3 feature is not enabled. \
    Enable the andor-sdk3 feature (or hardware in driver-andor) to use the real Andor SDK3.";

// ── Library management ──────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_InitialiseLibrary() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_FinaliseLibrary() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_InitialiseUtilityLibrary() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_FinaliseUtilityLibrary() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Device management ───────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_Open(_device_index: c_int, _handle: *mut AT_H) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_Close(_handle: AT_H) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Integer features ────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_SetInt(_handle: AT_H, _feature: *const AT_WC, _value: AT_64) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetInt(_handle: AT_H, _feature: *const AT_WC, _value: *mut AT_64) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetIntMax(_handle: AT_H, _feature: *const AT_WC, _value: *mut AT_64) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetIntMin(_handle: AT_H, _feature: *const AT_WC, _value: *mut AT_64) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Float features ──────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_SetFloat(_handle: AT_H, _feature: *const AT_WC, _value: c_double) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetFloat(_handle: AT_H, _feature: *const AT_WC, _value: *mut c_double) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetFloatMax(_handle: AT_H, _feature: *const AT_WC, _value: *mut c_double) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetFloatMin(_handle: AT_H, _feature: *const AT_WC, _value: *mut c_double) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Boolean features ────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_SetBool(_handle: AT_H, _feature: *const AT_WC, _value: AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetBool(_handle: AT_H, _feature: *const AT_WC, _value: *mut AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Enum features ───────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_SetEnumIndex(_handle: AT_H, _feature: *const AT_WC, _value: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetEnumIndex(_handle: AT_H, _feature: *const AT_WC, _value: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_GetEnumCount(_handle: AT_H, _feature: *const AT_WC, _count: *mut c_int) -> c_int {
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
pub unsafe extern "C" fn AT_IsEnumIndexAvailable(
    _handle: AT_H,
    _feature: *const AT_WC,
    _index: c_int,
    _available: *mut AT_BOOL,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_IsEnumIndexImplemented(
    _handle: AT_H,
    _feature: *const AT_WC,
    _index: c_int,
    _implemented: *mut AT_BOOL,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── String features ─────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_SetString(_handle: AT_H, _feature: *const AT_WC, _string: *const AT_WC) -> c_int {
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
pub unsafe extern "C" fn AT_GetStringMaxLength(
    _handle: AT_H,
    _feature: *const AT_WC,
    _max_string_length: *mut c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Command features ────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_Command(_handle: AT_H, _feature: *const AT_WC) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Feature query ───────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_IsImplemented(_handle: AT_H, _feature: *const AT_WC, _implemented: *mut AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_IsReadable(_handle: AT_H, _feature: *const AT_WC, _readable: *mut AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_IsWritable(_handle: AT_H, _feature: *const AT_WC, _writable: *mut AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_IsReadOnly(_handle: AT_H, _feature: *const AT_WC, _read_only: *mut AT_BOOL) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Buffer management ───────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_QueueBuffer(_handle: AT_H, _ptr: *mut AT_U8, _ptr_size: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_WaitBuffer(
    _handle: AT_H,
    _ptr: *mut *mut AT_U8,
    _ptr_size: *mut c_int,
    _timeout: c_uint,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_Flush(_handle: AT_H) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Feature callbacks ───────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_RegisterFeatureCallback(
    _handle: AT_H,
    _feature: *const AT_WC,
    _callback: FeatureCallback,
    _context: *mut c_void,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_UnregisterFeatureCallback(
    _handle: AT_H,
    _feature: *const AT_WC,
    _callback: FeatureCallback,
    _context: *mut c_void,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Buffer conversion (utility library) ─────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn AT_ConvertBuffer(
    _input: *mut AT_U8,
    _output: *mut AT_U8,
    _width: AT_64,
    _height: AT_64,
    _stride: AT_64,
    _input_pixel_encoding: *const AT_WC,
    _output_pixel_encoding: *const AT_WC,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn AT_ConvertBufferUsingMetadata(
    _input: *mut AT_U8,
    _output: *mut AT_U8,
    _imsize: AT_64,
    _output_pixel_encoding: *const AT_WC,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ══════════════════════════════════════════════════════════════════════════
// Spectrograph library (atspectrograph.dll / ShamrockCIF.h)
// ══════════════════════════════════════════════════════════════════════════

// ── Shamrock constants ──────────────────────────────────────────────────
pub const SHAMROCK_SUCCESS: c_int = 20202;
pub const SHAMROCK_COMMUNICATION_ERROR: c_int = 20201;
pub const SHAMROCK_P1INVALID: c_int = 20266;
pub const SHAMROCK_P2INVALID: c_int = 20267;
pub const SHAMROCK_P3INVALID: c_int = 20268;
pub const SHAMROCK_P4INVALID: c_int = 20269;
pub const SHAMROCK_NOT_INITIALIZED: c_int = 20275;
pub const SHAMROCK_NOT_AVAILABLE: c_int = 20292;

// Slit port indices
pub const SHAMROCK_INPUT_SLIT_SIDE: c_int = 1;
pub const SHAMROCK_INPUT_SLIT_DIRECT: c_int = 2;
pub const SHAMROCK_OUTPUT_SLIT_SIDE: c_int = 3;
pub const SHAMROCK_OUTPUT_SLIT_DIRECT: c_int = 4;

// Flipper mirror ports
pub const SHAMROCK_FLIPPER_INPUT: c_int = 1;
pub const SHAMROCK_FLIPPER_OUTPUT: c_int = 2;

// ── Library lifecycle ───────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockInitialize(_path: *mut c_char) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockClose() -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Device enumeration ──────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetNumberDevices(_num_devices: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetSerialNumber(_device: c_int, _serial: *mut c_char) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Grating control ────────────────────────────────────────────────────

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
pub unsafe extern "C" fn ShamrockGratingIsPresent(_device: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetGratingOffset(_device: c_int, _grating: c_int, _offset: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetGratingOffset(_device: c_int, _grating: c_int, _offset: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Wavelength control ─────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetWavelength(_device: c_int, _wavelength: f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetWavelength(_device: c_int, _wavelength: *mut f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetWavelengthLimits(
    _device: c_int,
    _grating: c_int,
    _min: *mut f32,
    _max: *mut f32,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockWavelengthReset(_device: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Slit control ────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockAutoSlitIsPresent(_device: c_int, _slit: c_int, _present: *mut c_int) -> c_int {
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
pub unsafe extern "C" fn ShamrockAutoSlitReset(_device: c_int, _slit: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSlitIsPresent(_device: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetSlit(_device: c_int, _width: f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetSlit(_device: c_int, _width: *mut f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Shutter control ────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetShutter(_device: c_int, _mode: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetShutter(_device: c_int, _mode: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockShutterIsPresent(_device: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Flipper mirror control ─────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetFlipperMirror(_device: c_int, _port: c_int, _pos: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetFlipperMirror(_device: c_int, _port: c_int, _pos: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockFlipperMirrorIsPresent(_device: c_int, _port: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockFlipperMirrorReset(_device: c_int, _port: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Calibration ─────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetCalibration(
    _device: c_int,
    _calibration: *mut f32,
    _num_pixels: c_int,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetPixelCalibrationCoefficients(
    _device: c_int,
    _a: *mut f32,
    _b: *mut f32,
    _c: *mut f32,
    _d: *mut f32,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Pixel / detector configuration ─────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetPixelWidth(_device: c_int, _width: f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetPixelWidth(_device: c_int, _width: *mut f32) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetNumberPixels(_device: c_int, _num_pixels: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetNumberPixels(_device: c_int, _num_pixels: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetDetectorOffset(_device: c_int, _offset: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetDetectorOffset(_device: c_int, _offset: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Optical parameters ─────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockEepromGetOpticalParams(
    _device: c_int,
    _focal_length: *mut f32,
    _angular_deviation: *mut f32,
    _focal_tilt: *mut f32,
) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Focus mirror control ────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockFocusMirrorIsPresent(_device: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetFocusMirror(_device: c_int, _focus: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetFocusMirror(_device: c_int, _focus: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetFocusMirrorMaxSteps(_device: c_int, _steps: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Filter control ─────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockFilterIsPresent(_device: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetFilter(_device: c_int, _filter: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetFilter(_device: c_int, _filter: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetFilterInfo(_device: c_int, _filter: c_int, _info: *mut c_char) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

// ── Accessory control ──────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn ShamrockAccessoryIsPresent(_device: c_int, _present: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockGetAccessoryState(_device: c_int, _accessory: c_int, _state: *mut c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}

#[no_mangle]
pub unsafe extern "C" fn ShamrockSetAccessory(_device: c_int, _accessory: c_int, _state: c_int) -> c_int {
    panic!("{}", ANDOR_SDK3_PANIC_MSG);
}
"#;

    std::fs::write(out_path.join("bindings.rs"), dummy).expect("Couldn't write dummy bindings!");
}
