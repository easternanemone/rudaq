//! Low-level FFI bindings for the Linux Comedi library.
//!
//! This crate provides raw, unsafe bindings to the comedilib C library,
//! which is the user-space interface to Comedi (Control and Measurement
//! Device Interface) kernel drivers.
//!
//! # Comedi Overview
//!
//! Comedi is a collection of Linux kernel drivers for data acquisition (DAQ)
//! hardware. It supports a wide variety of devices from manufacturers like
//! National Instruments, Measurement Computing, Advantech, and others.
//!
//! # Safety
//!
//! All functions in this crate are `unsafe` as they are direct FFI bindings.
//! For a safe wrapper, use the `daq-driver-comedi` crate instead.
//!
//! # Features
//!
//! - `comedi-sdk`: Generate bindings from system comedilib headers.
//!   Without this feature, pre-defined bindings are used for cross-compilation.
//!
//! # Example (unsafe)
//!
//! ```no_run
//! use comedi_sys::*;
//! use std::ffi::CString;
//! use std::ptr;
//!
//! unsafe {
//!     let path = CString::new("/dev/comedi0").unwrap();
//!     let dev = comedi_open(path.as_ptr());
//!     if !dev.is_null() {
//!         let n_subdevices = comedi_get_n_subdevices(dev);
//!         println!("Device has {} subdevices", n_subdevices);
//!         comedi_close(dev);
//!     }
//! }
//! ```

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]
#![allow(unsafe_code)] // FFI bindings require unsafe

// Include the generated bindings
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Re-export constants with standard names when using SDK feature
// (bindgen prefixes enum variants with the enum name, but dummy bindings use flat names)
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_AI as COMEDI_SUBD_AI;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_AO as COMEDI_SUBD_AO;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_CALIB as COMEDI_SUBD_CALIB;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_COUNTER as COMEDI_SUBD_COUNTER;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_DI as COMEDI_SUBD_DI;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_DIO as COMEDI_SUBD_DIO;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_DO as COMEDI_SUBD_DO;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_MEMORY as COMEDI_SUBD_MEMORY;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_PROC as COMEDI_SUBD_PROC;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_PWM as COMEDI_SUBD_PWM;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_SERIAL as COMEDI_SUBD_SERIAL;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_TIMER as COMEDI_SUBD_TIMER;
#[cfg(feature = "comedi-sdk")]
pub use comedi_subdevice_type_COMEDI_SUBD_UNUSED as COMEDI_SUBD_UNUSED;

#[cfg(feature = "comedi-sdk")]
pub use comedi_io_direction_COMEDI_INPUT as COMEDI_INPUT;
#[cfg(feature = "comedi-sdk")]
pub use comedi_io_direction_COMEDI_OUTPUT as COMEDI_OUTPUT;

// CR_* helper functions for channel/range/aref packing
// These are C macros that bindgen cannot translate, so we implement them in Rust.
// Only provide these when using SDK feature (dummy bindings already have them).
#[cfg(feature = "comedi-sdk")]
use std::os::raw::c_uint;

/// Pack channel, range, and analog reference into a single value
#[cfg(feature = "comedi-sdk")]
#[inline]
pub fn CR_PACK(chan: c_uint, rng: c_uint, aref: c_uint) -> c_uint {
    ((aref & 0x03) << 24) | ((rng & 0xff) << 16) | (chan & 0xffff)
}

/// Pack channel, range, analog reference, and flags into a single value
#[cfg(feature = "comedi-sdk")]
#[inline]
pub fn CR_PACK_FLAGS(chan: c_uint, rng: c_uint, aref: c_uint, flags: c_uint) -> c_uint {
    CR_PACK(chan, rng, aref) | ((flags & 0x0f) << 26)
}

/// Extract channel from packed value
#[cfg(feature = "comedi-sdk")]
#[inline]
pub fn CR_CHAN(a: c_uint) -> c_uint {
    a & 0xffff
}

/// Extract range from packed value
#[cfg(feature = "comedi-sdk")]
#[inline]
pub fn CR_RANGE(a: c_uint) -> c_uint {
    (a >> 16) & 0xff
}

/// Extract analog reference from packed value
#[cfg(feature = "comedi-sdk")]
#[inline]
pub fn CR_AREF(a: c_uint) -> c_uint {
    (a >> 24) & 0x03
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        // Verify subdevice type constants are defined
        assert_eq!(COMEDI_SUBD_AI, 1);
        assert_eq!(COMEDI_SUBD_AO, 2);
        assert_eq!(COMEDI_SUBD_DIO, 5);
        assert_eq!(COMEDI_SUBD_COUNTER, 6);
    }

    #[test]
    fn test_aref_constants() {
        // Verify analog reference constants
        assert_eq!(AREF_GROUND, 0);
        assert_eq!(AREF_COMMON, 1);
        assert_eq!(AREF_DIFF, 2);
    }

    #[test]
    fn test_cr_pack() {
        // Test channel/range/aref packing
        let packed = CR_PACK(0, 1, AREF_GROUND);
        assert_eq!(CR_CHAN(packed), 0);
        assert_eq!(CR_RANGE(packed), 1);
        assert_eq!(CR_AREF(packed), AREF_GROUND);
    }

    #[test]
    fn test_dio_constants() {
        assert_eq!(COMEDI_INPUT, 0);
        assert_eq!(COMEDI_OUTPUT, 1);
    }
}
