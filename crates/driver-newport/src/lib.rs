//! Newport hardware drivers for rust-daq.
//!
//! This crate provides drivers for Newport instruments, including:
//! - ESP300 Universal Motion Controller (RS-232)
//! - 1830-C Optical Power Meter (RS-232)
//!
//! # Usage
//!
//! Add to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! driver-newport = { path = "../driver-newport" }
//! ```
//!
//! Register the factories with your device registry:
//!
//! ```rust,ignore
//! use driver_newport::{Esp300Factory, Newport1830CFactory};
//!
//! registry.register_factory(Box::new(Esp300Factory));
//! registry.register_factory(Box::new(Newport1830CFactory));
//! ```

pub mod esp300;
pub mod newport_1830c;

pub use esp300::{Esp300Driver, Esp300Factory};
pub use newport_1830c::{Newport1830CDriver, Newport1830CFactory};

/// Force the linker to include this crate.
///
/// Call this function from main() to ensure the driver factories are
/// linked into the final binary and not stripped by the linker.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<Esp300Factory>());
    std::hint::black_box(std::any::TypeId::of::<Newport1830CFactory>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_does_not_panic() {
        link();
    }
}
