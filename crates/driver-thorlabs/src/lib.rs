//! Thorlabs hardware drivers for rust-daq.
//!
//! This crate provides drivers for Thorlabs devices, including:
//! - ELL14 Rotation Mount (RS-485 bus)
//!
//! # Usage
//!
//! Add to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! driver-thorlabs = { path = "../driver-thorlabs" }
//! ```
//!
//! Register the factory with your device registry:
//!
//! ```rust,ignore
//! use driver_thorlabs::Ell14Factory;
//!
//! registry.register_factory(Box::new(Ell14Factory));
//! ```

mod ell14;
pub mod shared_ports;

pub use ell14::{Ell14Driver, Ell14Factory};
pub use shared_ports::{get_or_open_port, SharedPort};

/// Force the linker to include this crate.
///
/// Call this function from main() to ensure the driver factories are
/// linked into the final binary and not stripped by the linker.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<Ell14Factory>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_does_not_panic() {
        link();
    }
}
