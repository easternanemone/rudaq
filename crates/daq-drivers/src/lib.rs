//! Driver Metacrate for rust-daq
//!
//! This crate aggregates all driver crates and provides unified feature flags.
//! It serves as the single dependency for applications that need hardware drivers.
//!
//! # Feature Flags
//!
//! ## Individual Drivers
//!
//! | Feature | Description | Crate |
//! |---------|-------------|-------|
//! | `pvcam` | PVCAM camera (mock mode) | `daq-driver-pvcam` |
//! | `pvcam_sdk` | PVCAM camera (real SDK) | `daq-driver-pvcam` |
//! | `comedi` | Comedi DAQ (mock mode) | `daq-driver-comedi` |
//! | `comedi_hardware` | Comedi DAQ (real hardware) | `daq-driver-comedi` |
//!
//! ## Convenience Sets
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `all` | All drivers with mock implementations |
//! | `maitai` | Hardware configuration for the maitai lab |
//! | `hardware` | All drivers with real hardware support |
//!
//! # Usage
//!
//! Add to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! daq-drivers = { path = "../daq-drivers", features = ["maitai"] }
//! ```
//!
//! Then in your `main.rs`, call `link_drivers()` early to ensure all driver
//! registrations are linked into the binary:
//!
//! ```rust,ignore
//! use daq_drivers::link_drivers;
//!
//! fn main() {
//!     // Ensure driver factories are linked
//!     link_drivers();
//!
//!     // ... rest of application
//! }
//! ```
//!
//! # Why `link_drivers()`?
//!
//! In Rust, code that is not directly referenced may be optimized away by the
//! linker. Driver crates register their factories at module initialization time,
//! but if nothing in the binary directly uses the crate, the linker may strip it.
//!
//! `link_drivers()` provides an explicit reference to each enabled driver crate,
//! ensuring their factory registrations are included in the final binary.
//!
//! # Driver Registration Flow
//!
//! ```text
//! 1. Binary starts
//! 2. main() calls daq_drivers::link_drivers()
//! 3. link_drivers() calls link() on each enabled driver crate
//! 4. Driver crates register their DriverFactory with the registry
//! 5. TOML config is loaded, drivers are instantiated via factories
//! ```

// Re-export core types for convenience
pub use common::driver::{Capability, DeviceComponents, DeviceMetadata, DriverFactory};

// =============================================================================
// Driver Crate Re-exports
// =============================================================================

/// PVCAM camera driver (Photometrics)
#[cfg(feature = "pvcam")]
pub use daq_driver_pvcam;

/// Comedi DAQ driver (Linux DAQ boards)
#[cfg(feature = "comedi")]
pub use daq_driver_comedi;

/// Mock drivers for testing and simulation
#[cfg(feature = "mock")]
pub use daq_driver_mock;

/// Thorlabs ELL14 rotation mount driver
#[cfg(feature = "thorlabs")]
pub use daq_driver_thorlabs;

/// Newport ESP300 motion controller and 1830-C power meter
#[cfg(feature = "newport")]
pub use daq_driver_newport;

/// Spectra-Physics MaiTai Ti:Sapphire laser
#[cfg(feature = "spectra_physics")]
pub use daq_driver_spectra_physics;

// =============================================================================
// Linker Reference Functions
// =============================================================================

/// Force the linker to include all enabled driver crates.
///
/// Call this function early in `main()` to ensure driver factory registrations
/// are not stripped by the linker. Without this, drivers may silently fail to
/// register if they are not directly used by other code in the binary.
///
/// # Example
///
/// ```rust,ignore
/// fn main() -> anyhow::Result<()> {
///     // Ensure all driver factories are linked
///     daq_drivers::link_drivers();
///
///     // Initialize tracing, load config, start server...
///     Ok(())
/// }
/// ```
///
/// # How It Works
///
/// Each driver crate exposes a `link()` function that references something
/// in the crate (typically the factory registration). This function calls
/// `link()` on each enabled driver crate, creating a reference chain that
/// prevents the linker from stripping the crate.
#[inline(never)]
pub fn link_drivers() {
    // PVCAM camera
    #[cfg(feature = "pvcam")]
    daq_driver_pvcam::link();

    // Comedi DAQ
    #[cfg(feature = "comedi")]
    daq_driver_comedi::link();

    // Mock drivers
    #[cfg(feature = "mock")]
    daq_driver_mock::link();

    // Thorlabs ELL14
    #[cfg(feature = "thorlabs")]
    daq_driver_thorlabs::link();

    // Newport ESP300 and 1830-C
    #[cfg(feature = "newport")]
    daq_driver_newport::link();

    // Spectra-Physics MaiTai
    #[cfg(feature = "spectra_physics")]
    daq_driver_spectra_physics::link();
}

/// Get a list of driver types that are linked into this binary.
///
/// This is useful for debugging and introspection. It returns the driver
/// type names (as used in TOML config) for all enabled drivers.
///
/// # Example
///
/// ```rust,ignore
/// for driver in daq_drivers::available_drivers() {
///     println!("Available driver: {}", driver);
/// }
/// ```
pub fn available_drivers() -> Vec<&'static str> {
    #[allow(unused_mut)] // mut needed when features enable push() calls
    let mut drivers = Vec::new();

    #[cfg(feature = "pvcam")]
    drivers.push("pvcam");

    #[cfg(feature = "comedi")]
    drivers.push("comedi");

    #[cfg(feature = "mock")]
    {
        drivers.push("mock_stage");
        drivers.push("mock_power_meter");
        drivers.push("mock_camera");
    }

    #[cfg(feature = "thorlabs")]
    drivers.push("ell14");

    #[cfg(feature = "newport")]
    {
        drivers.push("esp300");
        drivers.push("newport_1830c");
    }

    #[cfg(feature = "spectra_physics")]
    drivers.push("maitai");

    drivers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_drivers_does_not_panic() {
        // Just verify that link_drivers() can be called without panicking
        link_drivers();
    }

    #[test]
    fn test_available_drivers() {
        let drivers = available_drivers();
        // Should return a list (possibly empty if no features enabled)
        // Just verify it doesn't panic and returns a valid vector
        let _ = drivers;
    }
}
