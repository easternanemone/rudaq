//! Spectra-Physics hardware drivers for rust-daq.
//!
//! This crate provides drivers for Spectra-Physics instruments, including:
//! - MaiTai Ti:Sapphire Tunable Laser (RS-232/USB-to-USB)
//!
//! # Usage
//!
//! Add to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! driver-spectra-physics = { path = "../driver-spectra-physics" }
//! ```
//!
//! Register the factories with your device registry:
//!
//! ```rust,ignore
//! use driver_spectra_physics::MaiTaiFactory;
//!
//! registry.register_factory(Box::new(MaiTaiFactory));
//! ```

mod maitai;

pub use maitai::{MaiTaiDriver, MaiTaiFactory};

/// Force the linker to include this crate.
///
/// Call this function from main() to ensure the driver factories are
/// linked into the final binary and not stripped by the linker.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<MaiTaiFactory>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_does_not_panic() {
        link();
    }
}
