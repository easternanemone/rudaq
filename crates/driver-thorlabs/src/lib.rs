//! Thorlabs hardware drivers for rust-daq.
//!
//! This crate provides shared infrastructure for Thorlabs devices.
//! The ELL14 rotation mount driver has been superseded by the
//! driver-universal TOML manifest at `config/devices/ell14.toml`.
//!
//! The `shared_ports` module provides RS-485 bus port sharing, which
//! may be used by driver-universal for multi-device serial buses.

pub mod shared_ports;

pub use shared_ports::{get_or_open_port, SharedPort};

/// Force the linker to include this crate.
#[inline(never)]
pub fn link() {
    // No factories to link — ELL14 is now driver-universal
}
