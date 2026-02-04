//! Build script for dover-motion-sys FFI bindings.
//!
//! This script generates Rust FFI bindings from the Dover Motion MotionSynergyAPI
//! C++ headers using bindgen. It supports two modes:
//!
//! 1. With `dover-sdk` feature: Generates bindings from SDK headers
//! 2. Without feature: Uses pre-generated dummy bindings for cross-compilation
//!
//! # Platform Support
//!
//! ## Windows (Primary)
//! - Requires MotionSynergyAPI SDK installed
//! - Links against MotionSynergyCore.lib (static) or MotionSynergyCore.dll (dynamic)
//! - Default SDK path: C:\Program Files\Dover Motion\MotionSynergyAPI
//! - Environment variables:
//!   - DOVER_SDK_DIR: SDK installation directory
//!   - DOVER_INCLUDE_DIR: Header files directory (default: $DOVER_SDK_DIR/include)
//!   - DOVER_LIB_DIR: Library files directory (default: $DOVER_SDK_DIR/lib)
//!
//! ## Linux (Secondary)
//! - Requires libMotionSynergyCore.so installed
//! - Default library path: /usr/local/lib
//! - Default include path: /usr/local/include/dover-motion
//!
//! # Windows Wide String Handling
//!
//! Many Windows SDK functions use WCHAR (UTF-16). The generated bindings
//! will include these types. Use the `widestring` crate for safe conversion
//! if needed by the driver implementation.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.hpp");
    println!("cargo:rerun-if-env-changed=DOVER_SDK_DIR");
    println!("cargo:rerun-if-env-changed=DOVER_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=DOVER_LIB_DIR");

    #[cfg(feature = "dover-sdk")]
    {
        generate_bindings();
        link_library();
    }

    #[cfg(not(feature = "dover-sdk"))]
    generate_dummy_bindings();
}

#[cfg(feature = "dover-sdk")]
fn generate_bindings() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");

    // Determine include directory based on platform
    let include_dir = env::var("DOVER_INCLUDE_DIR").unwrap_or_else(|_| {
        if let Ok(sdk_dir) = env::var("DOVER_SDK_DIR") {
            format!("{}/include", sdk_dir)
        } else {
            match target_os.as_str() {
                "windows" => {
                    // Default Windows SDK path
                    "C:\\Program Files\\Dover Motion\\MotionSynergyAPI\\include".to_string()
                }
                "linux" => {
                    // Default Linux include path
                    "/usr/local/include/dover-motion".to_string()
                }
                _ => panic!(
                    "Unsupported target OS: {}. Dover Motion SDK only supports Windows and Linux.",
                    target_os
                ),
            }
        }
    });

    println!("cargo:rerun-if-changed={}/IAxisDevice.h", include_dir);
    println!("cargo:rerun-if-changed={}/MotionSynergyAPI.h", include_dir);
    println!(
        "cargo:rerun-if-changed={}/CommunicationSettings.h",
        include_dir
    );

    // Configure bindgen for C++ headers
    let mut builder = bindgen::Builder::default()
        .header("wrapper.hpp")
        .clang_arg(format!("-I{}", include_dir))
        // Enable C++17 support (required by Dover Motion SDK)
        .clang_arg("-std=c++17")
        // Allow all imp namespace types and functions
        .allowlist_namespace("imp")
        // Allow specific classes from the API manual
        .allowlist_type("imp::IAxisDevice")
        .allowlist_type("imp::MotionSynergyAPI")
        .allowlist_type("imp::CommunicationSettings")
        .allowlist_type("imp::IMotionControllerConfiguration")
        .allowlist_type("imp::MotionErrorSettings")
        .allowlist_type("imp::MotionTrackingSettings")
        // Allow critical functions for LIBS experiments
        .allowlist_function(".*Configure.*")
        .allowlist_function(".*Connect.*")
        .allowlist_function(".*Initialize.*")
        .allowlist_function(".*Shutdown.*")
        .allowlist_function(".*MoveAbsolute.*")
        .allowlist_function(".*MoveRelative.*")
        .allowlist_function(".*Stop.*")
        .allowlist_function(".*Home.*")
        .allowlist_function(".*GetActualPosition.*")
        .allowlist_function(".*GetCommandedPosition.*")
        .allowlist_function(".*SetVelocity.*")
        .allowlist_function(".*SetAcceleration.*")
        .allowlist_function(".*SetDeceleration.*")
        .allowlist_function(".*EnableTriggerOnPosition.*")
        .allowlist_function(".*DisableTriggerOnPosition.*")
        // Derive common traits
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        // Parse block comments as doc comments
        .generate_comments(true)
        // Enable C++ support
        .enable_cxx_namespaces()
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    // Platform-specific configuration
    if target_os == "windows" {
        // Windows may require additional includes for WCHAR support
        builder = builder.clang_arg("-DWIN32").clang_arg("-D_WINDOWS");
    }

    let bindings = builder
        .generate()
        .expect("Unable to generate Dover Motion bindings");

    let out_path = PathBuf::from(
        env::var("OUT_DIR").expect("OUT_DIR environment variable must be set by Cargo"),
    );
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

#[cfg(feature = "dover-sdk")]
fn link_library() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");

    // Determine library directory based on platform
    let lib_dir = env::var("DOVER_LIB_DIR").unwrap_or_else(|_| {
        if let Ok(sdk_dir) = env::var("DOVER_SDK_DIR") {
            format!("{}/lib", sdk_dir)
        } else {
            match target_os.as_str() {
                "windows" => "C:\\Program Files\\Dover Motion\\MotionSynergyAPI\\lib".to_string(),
                "linux" => "/usr/local/lib".to_string(),
                _ => panic!("Unsupported target OS: {}", target_os),
            }
        }
    });

    println!("cargo:rustc-link-search=native={}", lib_dir);

    match target_os.as_str() {
        "windows" => {
            // Link against MotionSynergyCore.lib (imports MotionSynergyCore.dll)
            println!("cargo:rustc-link-lib=dylib=MotionSynergyCore");

            // Inform user they need to copy the DLL
            eprintln!("NOTICE: After building, you must copy MotionSynergyCore.dll");
            eprintln!("        from {} to your executable directory", lib_dir);
            eprintln!("        or ensure it's in your system PATH.");
        }
        "linux" => {
            // Link against libMotionSynergyCore.so
            println!("cargo:rustc-link-lib=dylib=MotionSynergyCore");
        }
        _ => panic!("Unsupported target OS: {}", target_os),
    }
}

/// Generate dummy bindings when SDK is not available.
/// This allows the crate to compile on systems without Dover Motion SDK installed.
#[cfg(not(feature = "dover-sdk"))]
fn generate_dummy_bindings() {
    let out_path = PathBuf::from(
        env::var("OUT_DIR").expect("OUT_DIR environment variable must be set by Cargo"),
    );

    let dummy = r#"
// Dummy bindings - dover-sdk feature not enabled
//
// These are placeholder types and functions that allow the crate to compile
// without the actual Dover Motion SDK headers. Enable the `dover-sdk` feature
// to generate real bindings.

use std::os::raw::{c_char, c_int, c_uint, c_double, c_void};

/// Opaque handle to MotionSynergyAPI instance
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct MotionSynergyAPI {
    _unused: [u8; 0],
}

/// Opaque handle to IAxisDevice instance
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct IAxisDevice {
    _unused: [u8; 0],
}

/// Communication settings for an axis
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct CommunicationSettings {
    pub can_address: u16,
    pub can_port_baud_rate: c_uint,
    pub serial_address: u16,
    pub serial_port_baud_rate: c_uint,
    pub serial_port_protocol: c_uint,
}

/// Trigger types for trace capture
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TraceTrigger {
    Immediate = 0,
    OnMotionStart = 1,
    OnMotionStartNoEnd = 2,
    OnMotionStartDelayed = 3,
}

/// Active control modes
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ActiveControlMode {
    Normal = 0,
    UDPTime = 1,
    UDPSlave = 2,
    RCP = 3,
    ExternalControl = 4,
}

/// Digital output trigger modes
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DigitalOutputTrigger {
    None = 0,
    OnMotionStart = 1,
    OnMotionComplete = 2,
}

/// Movement structure
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct Movement {
    pub position: c_double,
    pub velocity: c_double,
    pub acceleration: c_double,
    pub deceleration: c_double,
    pub jerk: c_double,
}

// Panic stub implementations - these allow linking to succeed but will panic at runtime
// if called without the dover-sdk feature enabled.
//
// This is intentional: it allows the workspace to build and test on systems without
// Dover Motion SDK installed, while still catching any accidental usage at runtime.

const DOVER_SDK_PANIC_MSG: &str = "Dover Motion function called but dover-sdk feature is not enabled. \
    Enable the dover-sdk feature to use the real Dover Motion SDK library.";

// Note: Actual function stubs would be generated here based on the C++ API.
// For now, we provide type definitions only. Driver implementations will need
// to check for the dover-sdk feature before calling any functions.
"#;

    std::fs::write(out_path.join("bindings.rs"), dummy).expect("Couldn't write dummy bindings!");
}
