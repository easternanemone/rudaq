#[cfg(feature = "driver-thorlabs")]
pub mod ell14;

#[cfg(all(test, feature = "driver-thorlabs"))]
mod ell14_polling;

// Generic serial driver for config-driven devices
#[cfg(feature = "tokio_serial")]
pub mod generic_serial;

// Rhai scripting engine for config-driven drivers
#[cfg(feature = "scripting")]
pub mod script_engine;

// Re-export key types from generic_serial
#[cfg(feature = "tokio_serial")]
pub use generic_serial::{DynSerial, GenericSerialDriver, SharedPort};

// Re-export scripting types when enabled
#[cfg(feature = "scripting")]
pub use script_engine::{
    create_sandboxed_engine, execute_script_async, execute_script_sync, validate_script,
    CompiledScripts, ScriptContext, ScriptEngineConfig, ScriptResult,
};

#[cfg(feature = "driver-newport")]
pub mod esp300;
#[cfg(feature = "driver-spectra-physics")]
pub mod maitai;
pub mod mock;
#[cfg(feature = "serial")]
pub mod mock_serial;
#[cfg(feature = "driver-newport")]
pub mod newport_1830c;
#[cfg(feature = "driver_pvcam")]
pub use daq_driver_pvcam as pvcam;
