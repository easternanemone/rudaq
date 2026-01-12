//! Build script for comedi-sys FFI bindings.
//!
//! This script generates Rust FFI bindings from the comedilib C headers
//! using bindgen. It supports two modes:
//!
//! 1. With `comedi-sdk` feature: Generates bindings from system headers
//! 2. Without feature: Uses pre-generated bindings for cross-compilation

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-env-changed=COMEDI_INCLUDE_DIR");

    #[cfg(feature = "comedi-sdk")]
    generate_bindings();

    #[cfg(not(feature = "comedi-sdk"))]
    generate_dummy_bindings();

    // Link against comedi library when building for target with the SDK
    #[cfg(feature = "comedi-sdk")]
    {
        // Try pkg-config first
        if pkg_config::probe_library("comedilib").is_ok() {
            return;
        }

        // Fallback to standard locations
        println!("cargo:rustc-link-lib=comedi");

        // Check common library paths
        let lib_paths = ["/usr/local/lib", "/usr/lib", "/usr/lib/x86_64-linux-gnu"];

        for path in lib_paths {
            if std::path::Path::new(path).join("libcomedi.so").exists()
                || std::path::Path::new(path).join("libcomedi.a").exists()
            {
                println!("cargo:rustc-link-search=native={}", path);
                break;
            }
        }
    }
}

#[cfg(feature = "comedi-sdk")]
fn generate_bindings() {
    // Determine include directory
    let include_dir = env::var("COMEDI_INCLUDE_DIR").unwrap_or_else(|_| {
        // Try pkg-config
        if let Ok(lib) = pkg_config::probe_library("comedilib") {
            lib.include_paths
                .first()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/usr/local/include".to_string())
        } else {
            // Default locations
            for path in ["/usr/local/include", "/usr/include"] {
                if std::path::Path::new(path).join("comedilib.h").exists() {
                    return path.to_string();
                }
            }
            "/usr/local/include".to_string()
        }
    });

    println!("cargo:rerun-if-changed={}/comedilib.h", include_dir);
    println!("cargo:rerun-if-changed={}/comedi.h", include_dir);

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", include_dir))
        // Allow all comedi functions
        .allowlist_function("comedi_.*")
        // Allow all comedi types
        .allowlist_type("comedi_.*")
        .allowlist_type("lsampl_t")
        .allowlist_type("sampl_t")
        // Allow all comedi constants
        .allowlist_var("COMEDI_.*")
        .allowlist_var("AREF_.*")
        .allowlist_var("TRIG_.*")
        .allowlist_var("SDF_.*")
        .allowlist_var("INSN_.*")
        .allowlist_var("CMDF_.*")
        // Use default enum style to keep constants at top level (matches dummy bindings)
        .default_enum_style(bindgen::EnumVariation::Consts)
        // Derive common traits
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        // Parse block comments as doc comments
        .generate_comments(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate comedi bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

/// Generate dummy bindings when SDK is not available.
/// This allows the crate to compile on systems without comedilib installed.
#[cfg(not(feature = "comedi-sdk"))]
fn generate_dummy_bindings() {
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dummy = r#"
// Dummy bindings - comedi-sdk feature not enabled
//
// These are placeholder types and functions that allow the crate to compile
// without the actual comedilib headers. Enable the `comedi-sdk` feature
// to generate real bindings.

use std::os::raw::{c_char, c_int, c_uint, c_void};

/// Opaque handle to a comedi device
pub type comedi_t = c_void;

/// Large sample type (32-bit)
pub type lsampl_t = c_uint;

/// Small sample type (16-bit)
pub type sampl_t = u16;

/// Comedi range structure
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct comedi_range {
    pub min: f64,
    pub max: f64,
    pub unit: c_uint,
}

/// Comedi command structure for asynchronous I/O
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct comedi_cmd {
    pub subdev: c_uint,
    pub flags: c_uint,
    pub start_src: c_uint,
    pub start_arg: c_uint,
    pub scan_begin_src: c_uint,
    pub scan_begin_arg: c_uint,
    pub convert_src: c_uint,
    pub convert_arg: c_uint,
    pub scan_end_src: c_uint,
    pub scan_end_arg: c_uint,
    pub stop_src: c_uint,
    pub stop_arg: c_uint,
    pub chanlist: *mut c_uint,
    pub chanlist_len: c_uint,
    pub data: *mut sampl_t,
    pub data_len: c_uint,
}

// Subdevice types (must match bindgen's c_uint type)
pub const COMEDI_SUBD_UNUSED: c_uint = 0;
pub const COMEDI_SUBD_AI: c_uint = 1;
pub const COMEDI_SUBD_AO: c_uint = 2;
pub const COMEDI_SUBD_DI: c_uint = 3;
pub const COMEDI_SUBD_DO: c_uint = 4;
pub const COMEDI_SUBD_DIO: c_uint = 5;
pub const COMEDI_SUBD_COUNTER: c_uint = 6;
pub const COMEDI_SUBD_TIMER: c_uint = 7;
pub const COMEDI_SUBD_MEMORY: c_uint = 8;
pub const COMEDI_SUBD_CALIB: c_uint = 9;
pub const COMEDI_SUBD_PROC: c_uint = 10;
pub const COMEDI_SUBD_SERIAL: c_uint = 11;
pub const COMEDI_SUBD_PWM: c_uint = 12;

// Analog reference types
pub const AREF_GROUND: c_uint = 0;
pub const AREF_COMMON: c_uint = 1;
pub const AREF_DIFF: c_uint = 2;
pub const AREF_OTHER: c_uint = 3;

// Trigger sources
pub const TRIG_NONE: c_uint = 0x00000001;
pub const TRIG_NOW: c_uint = 0x00000002;
pub const TRIG_FOLLOW: c_uint = 0x00000004;
pub const TRIG_TIME: c_uint = 0x00000008;
pub const TRIG_TIMER: c_uint = 0x00000010;
pub const TRIG_COUNT: c_uint = 0x00000020;
pub const TRIG_EXT: c_uint = 0x00000040;
pub const TRIG_INT: c_uint = 0x00000080;
pub const TRIG_OTHER: c_uint = 0x00000100;

// Subdevice flags
pub const SDF_BUSY: c_uint = 0x0001;
pub const SDF_BUSY_OWNER: c_uint = 0x0002;
pub const SDF_LOCKED: c_uint = 0x0004;
pub const SDF_LOCK_OWNER: c_uint = 0x0008;
pub const SDF_MAXDATA: c_uint = 0x0010;
pub const SDF_FLAGS: c_uint = 0x0020;
pub const SDF_RANGETYPE: c_uint = 0x0040;
pub const SDF_MODE0: c_uint = 0x0080;
pub const SDF_MODE1: c_uint = 0x0100;
pub const SDF_MODE2: c_uint = 0x0200;
pub const SDF_MODE3: c_uint = 0x0400;
pub const SDF_MODE4: c_uint = 0x0800;
pub const SDF_CMD: c_uint = 0x1000;
pub const SDF_SOFT_CALIBRATED: c_uint = 0x2000;
pub const SDF_CMD_WRITE: c_uint = 0x4000;
pub const SDF_CMD_READ: c_uint = 0x8000;
pub const SDF_READABLE: c_uint = 0x00010000;
pub const SDF_WRITABLE: c_uint = 0x00020000;
pub const SDF_INTERNAL: c_uint = 0x00040000;
pub const SDF_GROUND: c_uint = 0x00100000;
pub const SDF_COMMON: c_uint = 0x00200000;
pub const SDF_DIFF: c_uint = 0x00400000;
pub const SDF_OTHER: c_uint = 0x00800000;
pub const SDF_DITHER: c_uint = 0x01000000;
pub const SDF_DEGLITCH: c_uint = 0x02000000;
pub const SDF_MMAP: c_uint = 0x04000000;
pub const SDF_RUNNING: c_uint = 0x08000000;
pub const SDF_LSAMPL: c_uint = 0x10000000;
pub const SDF_PACKED: c_uint = 0x20000000;

// Command flags
pub const CMDF_PRIORITY: c_uint = 0x00000008;
pub const CMDF_WRITE: c_uint = 0x00000040;
pub const CMDF_RAWDATA: c_uint = 0x00000080;

// Channel macros (as functions in Rust)
#[inline]
pub fn CR_PACK(chan: c_uint, rng: c_uint, aref: c_uint) -> c_uint {
    ((aref & 0x03) << 24) | ((rng & 0xff) << 16) | (chan & 0xffff)
}

#[inline]
pub fn CR_PACK_FLAGS(chan: c_uint, rng: c_uint, aref: c_uint, flags: c_uint) -> c_uint {
    CR_PACK(chan, rng, aref) | ((flags & 0x0f) << 26)
}

#[inline]
pub fn CR_CHAN(a: c_uint) -> c_uint {
    a & 0xffff
}

#[inline]
pub fn CR_RANGE(a: c_uint) -> c_uint {
    (a >> 16) & 0xff
}

#[inline]
pub fn CR_AREF(a: c_uint) -> c_uint {
    (a >> 24) & 0x03
}

// Dummy function declarations - these will fail at link time if actually called
// without the real library, which is the desired behavior

extern "C" {
    pub fn comedi_open(filename: *const c_char) -> *mut comedi_t;
    pub fn comedi_close(dev: *mut comedi_t) -> c_int;
    pub fn comedi_get_n_subdevices(dev: *mut comedi_t) -> c_int;
    pub fn comedi_get_subdevice_type(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_get_subdevice_flags(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_get_n_channels(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_get_n_ranges(dev: *mut comedi_t, subdevice: c_uint, channel: c_uint) -> c_int;
    pub fn comedi_get_maxdata(dev: *mut comedi_t, subdevice: c_uint, channel: c_uint) -> lsampl_t;
    pub fn comedi_get_range(
        dev: *mut comedi_t,
        subdevice: c_uint,
        channel: c_uint,
        range: c_uint,
    ) -> *mut comedi_range;
    pub fn comedi_get_board_name(dev: *mut comedi_t) -> *const c_char;
    pub fn comedi_get_driver_name(dev: *mut comedi_t) -> *const c_char;
    pub fn comedi_data_read(
        dev: *mut comedi_t,
        subdevice: c_uint,
        channel: c_uint,
        range: c_uint,
        aref: c_uint,
        data: *mut lsampl_t,
    ) -> c_int;
    pub fn comedi_data_write(
        dev: *mut comedi_t,
        subdevice: c_uint,
        channel: c_uint,
        range: c_uint,
        aref: c_uint,
        data: lsampl_t,
    ) -> c_int;
    pub fn comedi_dio_config(
        dev: *mut comedi_t,
        subdevice: c_uint,
        channel: c_uint,
        direction: c_uint,
    ) -> c_int;
    pub fn comedi_dio_read(
        dev: *mut comedi_t,
        subdevice: c_uint,
        channel: c_uint,
        bit: *mut c_uint,
    ) -> c_int;
    pub fn comedi_dio_write(
        dev: *mut comedi_t,
        subdevice: c_uint,
        channel: c_uint,
        bit: c_uint,
    ) -> c_int;
    pub fn comedi_dio_bitfield2(
        dev: *mut comedi_t,
        subdevice: c_uint,
        write_mask: c_uint,
        bits: *mut c_uint,
        base_channel: c_uint,
    ) -> c_int;
    pub fn comedi_to_phys(
        data: lsampl_t,
        range: *const comedi_range,
        maxdata: lsampl_t,
    ) -> f64;
    pub fn comedi_from_phys(
        data: f64,
        range: *const comedi_range,
        maxdata: lsampl_t,
    ) -> lsampl_t;
    pub fn comedi_command(dev: *mut comedi_t, cmd: *mut comedi_cmd) -> c_int;
    pub fn comedi_command_test(dev: *mut comedi_t, cmd: *mut comedi_cmd) -> c_int;
    pub fn comedi_cancel(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_poll(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_get_buffer_size(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_get_buffer_contents(dev: *mut comedi_t, subdevice: c_uint) -> c_int;
    pub fn comedi_mark_buffer_read(dev: *mut comedi_t, subdevice: c_uint, bytes: c_uint) -> c_int;
    pub fn comedi_fileno(dev: *mut comedi_t) -> c_int;
    pub fn comedi_strerror(errnum: c_int) -> *const c_char;
    pub fn comedi_errno() -> c_int;
    pub fn comedi_perror(s: *const c_char);
    pub fn comedi_loglevel(loglevel: c_int) -> c_int;
}

// DIO direction constants
pub const COMEDI_INPUT: c_uint = 0;
pub const COMEDI_OUTPUT: c_uint = 1;
"#;

    std::fs::write(out_path.join("bindings.rs"), dummy).expect("Couldn't write dummy bindings!");
}
