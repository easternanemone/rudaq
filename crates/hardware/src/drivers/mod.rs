// Re-export from standalone driver crates (bd-ha9c Driver Decoupling)
// New driver crates provide clean DriverFactory-based implementations

/// Mock drivers for testing (re-exported from driver-mock)
/// Note: Also available via `drivers::mock` module for backwards compatibility
pub use driver_mock as mock_drivers;

/// Thorlabs driver crate (DriverFactory-based)
#[cfg(feature = "thorlabs")]
pub use driver_thorlabs;

/// Newport driver crate (DriverFactory-based)
#[cfg(feature = "newport")]
pub use driver_newport;

/// Spectra-Physics driver crate (DriverFactory-based)
#[cfg(feature = "spectra_physics")]
pub use driver_spectra_physics;

// Binary protocol support (Modbus RTU, etc.)
pub mod binary_protocol;

// Re-export binary protocol types
pub use binary_protocol::{BinaryFrameBuilder, BinaryResponseParser, ParsedValue};

#[cfg(feature = "binary_protocol")]
pub use binary_protocol::{calculate_crc, validate_crc, CrcValue};

/// Mock drivers for testing (legacy module, re-exports from driver-mock)
pub mod mock;

/// Mock serial port for testing (local implementation)
#[cfg(feature = "serial")]
pub mod mock_serial;

#[cfg(feature = "comedi")]
pub use driver_comedi as comedi;
/// Newport 1830-C power meter (re-exported from driver-newport)
/// Note: The canonical implementation is in driver-newport crate.
#[cfg(feature = "newport")]
pub use driver_newport::newport_1830c;
#[cfg(feature = "pvcam")]
pub use driver_pvcam as pvcam;
