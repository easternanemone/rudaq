//! GenericDriver bindings for Rhai scripts.
//!
//! This module enables Rhai scripts to control any TOML-configured serial device
//! via the GenericSerialDriver from daq-hardware.
//!
//! # Quick Start
//!
//! ```rhai
//! // Load config and create driver
//! let driver = create_generic_driver(
//!     "config/devices/ell14.toml",
//!     "/dev/ttyUSB0",
//!     "2"
//! );
//!
//! // Use trait methods
//! driver.move_abs(45.0);
//! driver.wait_settled();
//! let pos = driver.position();
//! print("Position: " + pos);
//! ```
//!
//! # Available Methods
//!
//! ## Factory
//! - `create_generic_driver(config_path, port_path, address)` - Create driver
//!
//! ## Movable Trait
//! - `move_abs(position)` - Move to absolute position
//! - `move_rel(distance)` - Move relative amount
//! - `position()` - Get current position
//! - `wait_settled()` - Wait for motion to complete
//! - `stop()` - Emergency stop
//!
//! ## Readable Trait
//! - `read()` - Read value from device
//!
//! ## WavelengthTunable Trait
//! - `set_wavelength(nm)` - Set wavelength
//! - `get_wavelength()` - Get current wavelength
//!
//! ## ShutterControl Trait
//! - `open()` - Open shutter
//! - `close()` - Close shutter
//! - `is_open()` - Check if shutter is open
//!
//! ## Low-Level API
//! - `transaction(command)` - Send raw command, get response
//! - `send_command(command)` - Send command without response
//! - `format_command(cmd_name, params)` - Format command from config
//!
//! ## Parameters
//! - `get_param(name)` - Get device parameter
//! - `set_param(name, value)` - Set device parameter
//! - `address()` - Get device address
//!
//! ## Safety
//! - `set_soft_limits(min, max)` - Set software motion limits
//!
//! # Feature Flag
//!
//! Requires `generic_driver` feature (included in `scripting_full`).

use crate::{rhai_error, run_blocking, SoftLimits};
use common::capabilities::{Movable, Readable, ShutterControl, WavelengthTunable};
use common::serial::open_serial_async;
use daq_hardware::config::load_device_config;
use daq_hardware::drivers::generic_serial::{GenericSerialDriver, SharedPort};
use rhai::{Dynamic, Engine, EvalAltResult, Map, Position};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle for a GenericSerialDriver instance in Rhai
#[derive(Clone)]
pub struct GenericDriverHandle {
    pub driver: Arc<GenericSerialDriver>,
    pub soft_limits: SoftLimits,
    pub config_path: String,
}

impl GenericDriverHandle {
    pub fn new(driver: GenericSerialDriver, soft_limits: SoftLimits, config_path: String) -> Self {
        Self {
            driver: Arc::new(driver),
            soft_limits,
            config_path,
        }
    }
}

/// Register GenericSerialDriver types and functions with the Rhai engine
pub fn register_generic_driver_functions(engine: &mut Engine) {
    engine.register_type_with_name::<GenericDriverHandle>("GenericDriver");

    // Factory function: create_generic_driver(config_path, port, address)
    engine.register_fn(
        "create_generic_driver",
        |config_path: &str,
         port_path: &str,
         address: &str|
         -> Result<GenericDriverHandle, Box<EvalAltResult>> {
            let config_path = config_path.to_string();
            let port_path = port_path.to_string();
            let address = address.to_string();

            // Load config and create driver in blocking context
            run_blocking::<_, _, String>("create_generic_driver", async move {
                // 1. Load configuration
                let config = load_device_config(Path::new(&config_path))
                    .map_err(|e| format!("Failed to load config from {}: {}", config_path, e))?;

                // 2. Open serial port
                let baud_rate = config.connection.baud_rate;
                let serial_port = open_serial_async(&port_path, baud_rate, "GenericDevice")
                    .await
                    .map_err(|e| format!("Failed to open port {}: {}", port_path, e))?;

                // 3. Create SharedPort (unbuffered)
                // Note: GenericSerialDriver expects Arc<Mutex<DynSerial>>
                let shared_port: SharedPort = Arc::new(Mutex::new(Box::new(serial_port)));

                // 4. Create driver
                let driver = GenericSerialDriver::new(config, shared_port, &address)
                    .map_err(|e| format!("Failed to create driver: {}", e))?;

                // 5. Initialize device
                driver
                    .run_init_sequence()
                    .await
                    .map_err(|e| format!("Initialization failed: {}", e))?;

                Ok(GenericDriverHandle::new(
                    driver,
                    SoftLimits::unlimited(),
                    config_path,
                ))
            })
        },
    );

    // =========================================================================
    // Movable Trait Methods
    // =========================================================================

    engine.register_fn(
        "move_abs",
        |handle: &mut GenericDriverHandle, pos: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            if let Err(e) = handle.soft_limits.validate(pos) {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    e.into(),
                    Position::NONE,
                )));
            }

            run_blocking("move_abs", handle.driver.move_abs(pos))?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn(
        "move_rel",
        |handle: &mut GenericDriverHandle, dist: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            // Check soft limits if current position is known (best effort)
            // Note: This requires a read, which might be slow. For safety we should probably do it.
            if handle.soft_limits.min.is_some() || handle.soft_limits.max.is_some() {
                let current = run_blocking("position", handle.driver.position())?;
                if let Err(e) = handle.soft_limits.validate(current + dist) {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        e.into(),
                        Position::NONE,
                    )));
                }
            }

            run_blocking("move_rel", handle.driver.move_rel(dist))?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn(
        "position",
        |handle: &mut GenericDriverHandle| -> Result<f64, Box<EvalAltResult>> {
            run_blocking("position", handle.driver.position())
        },
    );

    engine.register_fn(
        "wait_settled",
        |handle: &mut GenericDriverHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("wait_settled", handle.driver.wait_settled())?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn(
        "stop",
        |handle: &mut GenericDriverHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("stop", handle.driver.stop())?;
            Ok(Dynamic::UNIT)
        },
    );

    // =========================================================================
    // Readable Trait Methods
    // =========================================================================

    engine.register_fn(
        "read",
        |handle: &mut GenericDriverHandle| -> Result<f64, Box<EvalAltResult>> {
            run_blocking("read", handle.driver.read())
        },
    );

    // =========================================================================
    // WavelengthTunable Trait Methods
    // =========================================================================

    engine.register_fn(
        "set_wavelength",
        |handle: &mut GenericDriverHandle, wl: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("set_wavelength", handle.driver.set_wavelength(wl))?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn(
        "get_wavelength",
        |handle: &mut GenericDriverHandle| -> Result<f64, Box<EvalAltResult>> {
            run_blocking("get_wavelength", handle.driver.get_wavelength())
        },
    );

    // =========================================================================
    // ShutterControl Trait Methods
    // =========================================================================

    engine.register_fn(
        "open",
        |handle: &mut GenericDriverHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("open", handle.driver.open_shutter())?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn(
        "close",
        |handle: &mut GenericDriverHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("close", handle.driver.close_shutter())?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn(
        "is_open",
        |handle: &mut GenericDriverHandle| -> Result<bool, Box<EvalAltResult>> {
            run_blocking("is_open", handle.driver.is_shutter_open())
        },
    );

    // =========================================================================
    // Low-level API & Parameters
    // =========================================================================

    engine.register_fn(
        "transaction",
        |handle: &mut GenericDriverHandle, command: &str| -> Result<String, Box<EvalAltResult>> {
            let command = command.to_string();
            run_blocking("transaction", handle.driver.transaction(&command))
        },
    );

    engine.register_fn(
        "send_command",
        |handle: &mut GenericDriverHandle, command: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            let command = command.to_string();
            run_blocking("send_command", handle.driver.send_command(&command))?;
            Ok(Dynamic::UNIT)
        },
    );

    // Format command with parameters: format_command("move_abs", #{pos: 100})
    engine.register_fn(
        "format_command",
        |handle: &mut GenericDriverHandle,
         template_name: &str,
         params: Map|
         -> Result<String, Box<EvalAltResult>> {
            // Convert Rhai Map (Map<String, Dynamic>) to HashMap<String, f64>
            // Note: This assumes all params are numbers. If strings are needed, GenericSerialDriver
            // might need updating or we need a more complex conversion here.
            let mut param_map = HashMap::new();
            for (k, v) in params {
                if let Some(f) = v.as_float().ok() {
                    param_map.insert(k.to_string(), f);
                } else if let Some(i) = v.as_int().ok() {
                    param_map.insert(k.to_string(), i as f64);
                }
            }

            let template_name = template_name.to_string();

            // format_command is async, so we must use run_blocking
            run_blocking("format_command", async move {
                handle
                    .driver
                    .format_command(&template_name, &param_map)
                    .await
            })
            .map_err(|e| rhai_error("format_command", e))
        },
    );

    engine.register_fn(
        "get_param",
        |handle: &mut GenericDriverHandle, name: &str| -> Result<f64, Box<EvalAltResult>> {
            let name = name.to_string();
            run_blocking("get_param", async move {
                handle
                    .driver
                    .get_parameter(&name)
                    .await
                    .ok_or_else(|| format!("Parameter '{}' not found", name))
            })
            .map_err(|e| rhai_error("get_param", e))
        },
    );

    engine.register_fn(
        "set_param",
        |handle: &mut GenericDriverHandle,
         name: &str,
         value: f64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let name = name.to_string();
            run_blocking::<_, _, String>("set_param", async move {
                handle.driver.set_parameter(&name, value).await;
                Ok(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    engine.register_fn("address", |handle: &mut GenericDriverHandle| -> String {
        handle.driver.address().to_string()
    });

    // =========================================================================
    // Safety
    // =========================================================================

    engine.register_fn(
        "set_soft_limits",
        |handle: &mut GenericDriverHandle, min: f64, max: f64| {
            handle.soft_limits = SoftLimits::new(min, max);
        },
    );
}
