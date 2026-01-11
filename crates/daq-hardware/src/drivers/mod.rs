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

#[cfg(feature = "newport")]
pub mod esp300;
#[cfg(feature = "spectra_physics")]
pub mod maitai;
pub mod mock;
#[cfg(feature = "serial")]
pub mod mock_serial;
#[cfg(feature = "newport")]
pub mod newport_1830c;
#[cfg(feature = "pvcam")]
pub use daq_driver_pvcam as pvcam;
#[cfg(feature = "comedi")]
pub use daq_driver_comedi as comedi;
