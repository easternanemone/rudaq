// Re-export from standalone driver crates (bd-ha9c Driver Decoupling)
// New driver crates provide clean DriverFactory-based implementations

/// Mock drivers for testing (re-exported from daq-driver-mock)
/// Note: Also available via `drivers::mock` module for backwards compatibility
pub use daq_driver_mock as mock_drivers;

/// Thorlabs driver crate (DriverFactory-based)
#[cfg(feature = "thorlabs")]
pub use daq_driver_thorlabs;

/// Newport driver crate (DriverFactory-based)
#[cfg(feature = "newport")]
pub use daq_driver_newport;

/// Spectra-Physics driver crate (DriverFactory-based)
#[cfg(feature = "spectra_physics")]
pub use daq_driver_spectra_physics;

// Legacy driver modules (kept for backwards compatibility)
// TODO: Migrate to DriverFactory-based crates (bd-ha9c Phase 10+)

#[cfg(feature = "thorlabs")]
pub mod ell14;

#[cfg(all(test, feature = "thorlabs"))]
mod ell14_polling;

// Generic serial driver for config-driven devices
#[cfg(feature = "serial")]
pub mod generic_serial;

// Rhai scripting engine for config-driven drivers
#[cfg(feature = "scripting")]
pub mod script_engine;

// Binary protocol support (Modbus RTU, etc.)
pub mod binary_protocol;

// Re-export key types from generic_serial
#[cfg(feature = "serial")]
pub use generic_serial::{DynSerial, GenericSerialDriver, SharedPort};

// Re-export scripting types when enabled
#[cfg(feature = "scripting")]
pub use script_engine::{
    create_sandboxed_engine, execute_script_async, execute_script_sync, validate_script,
    CompiledScripts, ScriptContext, ScriptEngineConfig, ScriptResult,
};

// Re-export binary protocol types
pub use binary_protocol::{BinaryFrameBuilder, BinaryResponseParser, ParsedValue};

#[cfg(feature = "binary_protocol")]
pub use binary_protocol::{calculate_crc, validate_crc, CrcValue};

/// Newport ESP300 motion controller (legacy module)
#[cfg(feature = "newport")]
pub mod esp300;

/// MaiTai Ti:Sapphire laser driver (legacy module)
#[cfg(feature = "spectra_physics")]
pub mod maitai;

/// Mock drivers for testing (legacy module, re-exports from daq-driver-mock)
pub mod mock;

/// Mock serial port for testing (local implementation)
#[cfg(feature = "serial")]
pub mod mock_serial;

#[cfg(feature = "comedi")]
pub use daq_driver_comedi as comedi;
/// Newport 1830-C power meter (re-exported from daq-driver-newport)
/// Note: The canonical implementation is in daq-driver-newport crate.
#[cfg(feature = "newport")]
pub use daq_driver_newport::newport_1830c;
#[cfg(feature = "pvcam")]
pub use daq_driver_pvcam as pvcam;
