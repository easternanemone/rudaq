// Hardware drivers module.
//
// Concrete driver crate re-exports have been moved to the `driver-registry` crate
// (bd-ga7k.10 dependency inversion). This module retains only:
// - Mock drivers (always compiled, used for testing)
// - Binary protocol support (local implementation)
// - Mock serial port (local implementation)
//
// For concrete drivers, depend directly on the driver crate:
//   use driver_comedi::ComediDevice;

/// Mock drivers for testing (re-exported from driver-mock)
/// Note: Also available via `drivers::mock` module for backwards compatibility
#[cfg(any(test, feature = "test-util"))]
pub use driver_mock as mock_drivers;

// Binary protocol support (Modbus RTU, etc.)
pub mod binary_protocol;

// Re-export binary protocol types
pub use binary_protocol::{BinaryFrameBuilder, BinaryResponseParser, ParsedValue};

#[cfg(feature = "binary_protocol")]
pub use binary_protocol::{CrcValue, calculate_crc, validate_crc};

/// Mock drivers for testing (legacy module, re-exports from driver-mock)
#[cfg(any(test, feature = "test-util"))]
pub mod mock;

/// Mock serial port for testing (local implementation)
#[cfg(feature = "serial")]
pub mod mock_serial;
