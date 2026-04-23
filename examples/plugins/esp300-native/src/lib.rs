//! ESP300 Native Plugin
//!
//! This example demonstrates how to create a native Rust plugin for rust-daq using
//! the `plugin-api` crate with `abi_stable` for FFI safety.
//!
//! # Features Demonstrated
//!
//! - `#[export_root_module]` macro for plugin entry point
//! - FFI-safe types: `RString`, `RVec`, `RHashMap`, `RResult`, `ROption`
//! - Full module lifecycle: configure, stage, start, stop, unstage
//! - Movable capability: move_abs, move_rel, position, wait_settled
//! - State serialization for hot-reload support
//! - Multi-axis coordination
//!
//! # Protocol Reference
//!
//! Newport ESP300 uses ASCII commands over RS-232:
//! - Format: `{Axis}{Command}{Value}`
//! - Example: `1PA5.0` (axis 1, position absolute, 5.0mm)
//! - Baud: 19200, 8N1
//!
//! # Building
//!
//! ```bash
//! cargo build --release
//! ```
//!
//! The output library will be at:
//! - Linux: `target/release/libesp300_native.so`
//! - macOS: `target/release/libesp300_native.dylib`
//! - Windows: `target/release/esp300_native.dll`

#![allow(clippy::new_without_default)]

use plugin_api::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, error, info, warn};

// =============================================================================
// Plugin Entry Point
// =============================================================================

/// Export the root module for plugin loading.
///
/// This is the entry point that the host application uses to load the plugin.
/// The `#[export_root_module]` macro from abi_stable handles the FFI machinery.
#[abi_stable::export_root_module]
fn get_root_module() -> PluginMod_Ref {
    PluginMod {
        abi_version,
        get_metadata,
        list_module_types,
        create_module,
    }
    .leak_into_prefix()
}

/// Returns the ABI version this plugin was compiled with.
///
/// The host will check this for compatibility before loading modules.
#[abi_stable::sabi_extern_fn]
fn abi_version() -> AbiVersion {
    AbiVersion::CURRENT
}

/// Returns plugin metadata for identification.
#[abi_stable::sabi_extern_fn]
fn get_metadata() -> PluginMetadata {
    PluginMetadata::new("esp300-native", "ESP300 Translation Stage", "1.0.0")
        .with_author("rust-daq Team")
        .with_description("Native Rust driver for Newport ESP300 motion controller")
        .with_module_type("esp300_stage")
        .with_min_daq_version("0.1.0")
}

/// Lists all module types provided by this plugin.
#[abi_stable::sabi_extern_fn]
fn list_module_types() -> RVec<FfiModuleTypeInfo> {
    let mut types = RVec::new();
    types.push(Esp300Stage::type_info_static());
    types
}

/// Creates a new module instance by type ID.
#[abi_stable::sabi_extern_fn]
fn create_module(type_id: RString) -> RResult<ModuleFfiBox, RString> {
    match type_id.as_str() {
        "esp300_stage" => {
            let module = Esp300Stage::new();
            // Convert to trait object using abi_stable's mechanism
            let boxed = ModuleFfi_TO::from_value(module, abi_stable::sabi_trait::TD_CanDowncast);
            RResult::ROk(boxed)
        }
        _ => RResult::RErr(RString::from(format!("Unknown module type: {}", type_id))),
    }
}

// =============================================================================
// State Types (for hot-reload serialization)
// =============================================================================

/// Serializable state for hot-reload support.
///
/// When the plugin is hot-reloaded during development, this state is serialized,
/// the old plugin is unloaded, the new plugin is loaded, and the state is restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Esp300State {
    /// Current position in mm
    pub position: f64,
    /// Target position for moves in progress
    pub target_position: Option<f64>,
    /// Current velocity setting in mm/s
    pub velocity: f64,
    /// Current acceleration setting in mm/s²
    pub acceleration: f64,
    /// Whether the stage has been homed
    pub is_homed: bool,
    /// Motion status
    pub is_moving: bool,
}

impl Default for Esp300State {
    fn default() -> Self {
        Self {
            position: 0.0,
            target_position: None,
            velocity: 10.0,
            acceleration: 50.0,
            is_homed: false,
            is_moving: false,
        }
    }
}

// =============================================================================
// ESP300 Stage Module
// =============================================================================

/// Newport ESP300 translation stage module.
///
/// This module implements the `ModuleFfi` trait to provide FFI-safe access
/// to the ESP300 motion controller functionality.
pub struct Esp300Stage {
    /// Module state
    state: FfiModuleState,
    /// Configuration parameters
    config: RHashMap<RString, RString>,
    /// Internal state (protected by mutex for thread safety)
    inner: Arc<Mutex<Esp300Inner>>,
    /// Pending events to be polled by the host
    events: Arc<Mutex<VecDeque<FfiModuleEvent>>>,
    /// Pending data points to be polled by the host
    data: Arc<Mutex<VecDeque<FfiModuleDataPoint>>>,
}

/// Internal state that requires mutual exclusion.
struct Esp300Inner {
    /// Serial port path
    port_path: String,
    /// Axis number (1-3)
    axis: u8,
    /// Serializable state
    state: Esp300State,
    /// Mock mode for testing
    mock_mode: bool,
    /// Active serial port connection
    port: Option<Box<dyn tokio_serial::SerialPort>>,
}

impl Esp300Stage {
    /// Creates a new ESP300 stage module.
    pub fn new() -> Self {
        Self {
            state: FfiModuleState::Created,
            config: RHashMap::new(),
            inner: Arc::new(Mutex::new(Esp300Inner {
                port_path: "/dev/ttyUSB0".to_string(),
                axis: 1,
                state: Esp300State::default(),
                mock_mode: cfg!(feature = "mock"),
                port: None,
            })),
            events: Arc::new(Mutex::new(VecDeque::new())),
            data: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Returns static type information for this module.
    fn type_info_static() -> FfiModuleTypeInfo {
        let mut event_types = RVec::new();
        event_types.push(RString::from("motion_started"));
        event_types.push(RString::from("motion_complete"));
        event_types.push(RString::from("position_update"));
        event_types.push(RString::from("error"));
        event_types.push(RString::from("homed"));

        let mut data_types = RVec::new();
        data_types.push(RString::from("position"));
        data_types.push(RString::from("velocity"));
        data_types.push(RString::from("status"));

        let mut required_roles = RVec::new();
        required_roles.push(FfiModuleRole {
            role_id: RString::from("serial_port"),
            description: RString::from("Serial port for ESP300 communication"),
            display_name: RString::from("Serial Port"),
            required_capability: RString::from("serial"),
            allows_multiple: false,
        });

        FfiModuleTypeInfo {
            type_id: RString::from("esp300_stage"),
            display_name: RString::from("ESP300 Translation Stage"),
            description: RString::from("Newport ESP300 multi-axis motion controller driver"),
            version: RString::from("1.0.0"),
            parameters: Self::parameters(),
            event_types,
            data_types,
            required_roles,
            optional_roles: RVec::new(),
        }
    }

    /// Returns parameter definitions for this module.
    fn parameters() -> RVec<FfiModuleParameter> {
        let mut params = RVec::new();

        params.push(FfiModuleParameter {
            param_id: RString::from("port_path"),
            display_name: RString::from("Serial Port"),
            description: RString::from("Path to the serial port (e.g., /dev/ttyUSB0, COM3)"),
            param_type: RString::from("string"),
            default_value: RString::from("/dev/ttyUSB0"),
            min_value: ROption::RNone,
            max_value: ROption::RNone,
            enum_values: RVec::new(),
            units: RString::new(),
            required: true,
        });

        params.push(FfiModuleParameter {
            param_id: RString::from("axis"),
            display_name: RString::from("Axis Number"),
            description: RString::from("ESP300 axis number (1-3)"),
            param_type: RString::from("int"),
            default_value: RString::from("1"),
            min_value: ROption::RSome(RString::from("1")),
            max_value: ROption::RSome(RString::from("3")),
            enum_values: RVec::new(),
            units: RString::new(),
            required: true,
        });

        params.push(FfiModuleParameter {
            param_id: RString::from("velocity"),
            display_name: RString::from("Velocity"),
            description: RString::from("Motion velocity"),
            param_type: RString::from("float"),
            default_value: RString::from("10.0"),
            min_value: ROption::RSome(RString::from("0.1")),
            max_value: ROption::RSome(RString::from("100.0")),
            enum_values: RVec::new(),
            units: RString::from("mm/s"),
            required: false,
        });

        params.push(FfiModuleParameter {
            param_id: RString::from("acceleration"),
            display_name: RString::from("Acceleration"),
            description: RString::from("Motion acceleration"),
            param_type: RString::from("float"),
            default_value: RString::from("50.0"),
            min_value: ROption::RSome(RString::from("1.0")),
            max_value: ROption::RSome(RString::from("500.0")),
            enum_values: RVec::new(),
            units: RString::from("mm/s²"),
            required: false,
        });

        params
    }

    /// Emits an event to the event queue.
    fn emit_event(&self, event_type: &str, message: &str, severity: u8) {
        let event = FfiModuleEvent {
            event_type: RString::from(event_type),
            severity,
            message: RString::from(message),
            data: RHashMap::new(),
        };
        if let Ok(mut events) = self.events.lock() {
            events.push_back(event);
        }
    }

    /// Emits a data point to the data queue.
    fn emit_data(&self, data_type: &str, values: &[(String, f64)]) {
        let mut value_map = RHashMap::new();
        for (key, val) in values {
            value_map.insert(RString::from(key.as_str()), *val);
        }
        let data = FfiModuleDataPoint {
            data_type: RString::from(data_type),
            timestamp_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            values: value_map,
            metadata: RHashMap::new(),
        };
        if let Ok(mut data_queue) = self.data.lock() {
            data_queue.push_back(data);
        }
    }

    // =========================================================================
    // Movable Capability Implementation
    // =========================================================================

    /// Moves the stage to an absolute position.
    ///
    /// This demonstrates how to implement the Movable capability.
    /// In a real implementation, this would send serial commands to the ESP300.
    pub fn move_abs(&self, position: f64) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        if !inner.state.is_homed && !inner.mock_mode {
            return Err("Stage must be homed before moving".to_string());
        }

        info!(
            "ESP300[{}]: Moving to position {:.3} mm",
            inner.axis, position
        );

        if inner.mock_mode {
            // Simulate motion in mock mode
            inner.state.target_position = Some(position);
            inner.state.is_moving = true;
            // Simulate instant completion for mock
            std::thread::sleep(Duration::from_millis(10));
            inner.state.position = position;
            inner.state.is_moving = false;
            inner.state.target_position = None;
        } else {
            // Real implementation would send: "{axis}PA{position}\r\n"
            let cmd = format!("{}PA{:.6}\r\n", inner.axis, position);
            if let Some(port) = &mut inner.port {
                if let Err(e) = port.write_all(cmd.as_bytes()) {
                    return Err(format!("Serial write error: {}", e));
                }
            } else {
                warn!("ESP300: Serial port not open, dropping command: {}", cmd.trim());
            }
            inner.state.target_position = Some(position);
            inner.state.is_moving = true;
        }

        drop(inner);

        self.emit_event(
            "motion_started",
            &format!("Moving to {:.3} mm", position),
            1,
        );
        self.emit_data("position", &[("target".to_string(), position)]);

        Ok(())
    }

    /// Moves the stage by a relative distance.
    pub fn move_rel(&self, distance: f64) -> Result<(), String> {
        let inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let current = inner.state.position;
        drop(inner);

        self.move_abs(current + distance)
    }

    /// Returns the current position.
    pub fn position(&self) -> Result<f64, String> {
        let inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        if inner.mock_mode {
            Ok(inner.state.position)
        } else {
            // Real implementation would query: "{axis}TP?\r\n"
            Ok(inner.state.position)
        }
    }

    /// Waits for motion to complete.
    pub fn wait_settled(&self) -> Result<(), String> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(60);

        loop {
            if start.elapsed() > timeout {
                return Err("Timeout waiting for motion to settle".to_string());
            }

            let inner = self
                .inner
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;

            if !inner.state.is_moving {
                // Update final position
                let pos = inner.state.position;
                drop(inner);

                self.emit_event("motion_complete", &format!("Settled at {:.3} mm", pos), 1);
                self.emit_data("position", &[("position".to_string(), pos)]);

                return Ok(());
            }

            if inner.mock_mode {
                // Mock mode: complete motion immediately
                drop(inner);
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|e| format!("Lock error: {}", e))?;
                if let Some(target) = inner.state.target_position {
                    inner.state.position = target;
                }
                inner.state.is_moving = false;
                inner.state.target_position = None;
                continue;
            }

            drop(inner);

            // Poll at 100ms intervals
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Stops motion immediately.
    pub fn stop_motion(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        info!("ESP300[{}]: Stopping motion", inner.axis);

        if inner.mock_mode {
            inner.state.is_moving = false;
            inner.state.target_position = None;
        } else {
            // Real implementation would send: "{axis}ST\r\n"
            let cmd = format!("{}ST\r\n", inner.axis);
            if let Some(port) = &mut inner.port {
                if let Err(e) = port.write_all(cmd.as_bytes()) {
                    error!("ESP300: Serial write error: {}", e);
                }
            } else {
                warn!("ESP300: Serial port not open, dropping command: {}", cmd.trim());
            }
        }

        drop(inner);

        self.emit_event("motion_complete", "Motion stopped by user", 2);

        Ok(())
    }

    /// Homes the stage (finds mechanical zero).
    pub fn home(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        info!("ESP300[{}]: Homing axis", inner.axis);

        if inner.mock_mode {
            // Simulate homing
            std::thread::sleep(Duration::from_millis(100));
            inner.state.position = 0.0;
            inner.state.is_homed = true;
        } else {
            // Real implementation would send: "{axis}OR\r\n"
            let cmd = format!("{}OR\r\n", inner.axis);
            if let Some(port) = &mut inner.port {
                if let Err(e) = port.write_all(cmd.as_bytes()) {
                    error!("ESP300: Serial write error: {}", e);
                }
            } else {
                warn!("ESP300: Serial port not open, dropping command: {}", cmd.trim());
            }
        }

        drop(inner);

        self.emit_event("homed", "Axis homed successfully", 1);

        Ok(())
    }

    /// Serializes state for hot-reload.
    pub fn serialize_state(&self) -> Result<String, String> {
        let inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        serde_json::to_string(&inner.state).map_err(|e| format!("Serialization error: {}", e))
    }

    /// Restores state after hot-reload.
    pub fn restore_state(&self, state_json: &str) -> Result<(), String> {
        let state: Esp300State = serde_json::from_str(state_json)
            .map_err(|e| format!("Deserialization error: {}", e))?;

        let mut inner = self
            .inner
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        inner.state = state;

        info!("ESP300: State restored from hot-reload");

        Ok(())
    }
}

// =============================================================================
// ModuleFfi Trait Implementation
// =============================================================================

impl ModuleFfi for Esp300Stage {
    fn type_info(&self) -> FfiModuleTypeInfo {
        Self::type_info_static()
    }

    fn type_id(&self) -> RString {
        RString::from("esp300_stage")
    }

    fn state(&self) -> FfiModuleState {
        self.state
    }

    fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>> {
        let mut warnings = RVec::new();

        // Parse port_path
        if let Some(value) = params.get(&RString::from("port_path")) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.port_path = value.to_string();
                debug!("ESP300: Configured port_path = {}", value);
            }
            self.config
                .insert(RString::from("port_path"), value.clone());
        }

        // Parse axis
        if let Some(value) = params.get(&RString::from("axis")) {
            if let Ok(axis) = value.parse::<u8>() {
                if (1..=3).contains(&axis) {
                    if let Ok(mut inner) = self.inner.lock() {
                        inner.axis = axis;
                        debug!("ESP300: Configured axis = {}", axis);
                    }
                } else {
                    return RResult::RErr(RString::from("Axis must be between 1 and 3"));
                }
            } else {
                return RResult::RErr(RString::from("Invalid axis value"));
            }
            self.config.insert(RString::from("axis"), value.clone());
        }

        // Parse velocity
        if let Some(value) = params.get(&RString::from("velocity")) {
            if let Ok(vel) = value.parse::<f64>() {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.state.velocity = vel;
                    debug!("ESP300: Configured velocity = {} mm/s", vel);
                }
            } else {
                warnings.push(RString::from(format!(
                    "Invalid velocity '{}', using default",
                    value
                )));
            }
            self.config.insert(RString::from("velocity"), value.clone());
        }

        // Parse acceleration
        if let Some(value) = params.get(&RString::from("acceleration")) {
            if let Ok(accel) = value.parse::<f64>() {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.state.acceleration = accel;
                    debug!("ESP300: Configured acceleration = {} mm/s²", accel);
                }
            } else {
                warnings.push(RString::from(format!(
                    "Invalid acceleration '{}', using default",
                    value
                )));
            }
            self.config
                .insert(RString::from("acceleration"), value.clone());
        }

        // Parse mock mode
        if let Some(value) = params.get(&RString::from("mock")) {
            if let Ok(mock) = value.parse::<bool>() {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.mock_mode = mock;
                    debug!("ESP300: Mock mode = {}", mock);
                }
            }
            self.config.insert(RString::from("mock"), value.clone());
        }

        self.state = FfiModuleState::Configured;
        info!("ESP300: Module configured");

        RResult::ROk(warnings)
    }

    fn get_config(&self) -> FfiModuleConfig {
        self.config.clone()
    }

    fn stage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()> {
        info!(
            "ESP300: Staging module (instance: {})",
            ctx.module_id.as_str()
        );

        // In a real implementation, this would:
        // 1. Open the serial port
        // 2. Initialize communication
        // 3. Query device status

        let inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(e) => return RResult::RErr(RString::from(format!("Lock error: {}", e))),
        };

        if !inner.mock_mode {
            match tokio_serial::new(&inner.port_path, 19200)
                .timeout(Duration::from_millis(100))
                .open()
            {
                Ok(port) => {
                    inner.port = Some(port);
                    debug!(
                        "ESP300: Opened port {} for axis {}",
                        inner.port_path, inner.axis
                    );
                }
                Err(e) => {
                    error!("ESP300: Failed to open port {}: {}", inner.port_path, e);
                    // Return early so we don't stage
                    return RResult::RErr(RString::from(format!("Failed to open port: {}", e)));
                }
            }
        }

        drop(inner);

        self.state = FfiModuleState::Staged;
        self.emit_event("staged", "Module staged and ready", 1);

        RResult::ROk(())
    }

    fn unstage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()> {
        info!(
            "ESP300: Unstaging module (instance: {})",
            ctx.module_id.as_str()
        );

        // In a real implementation, this would:
        // 1. Stop any motion
        // 2. Close the serial port
        // 3. Clean up resources

        // Try to stop motion if running
        let _ = self.stop_motion();

        if let Ok(mut inner) = self.inner.lock() {
            // Drop port to close it
            inner.port = None;
        }

        self.state = FfiModuleState::Configured;
        self.emit_event("unstaged", "Module unstaged", 1);

        RResult::ROk(())
    }

    fn start(&mut self, ctx: FfiModuleContext) -> FfiModuleResult<()> {
        info!(
            "ESP300: Starting module (instance: {})",
            ctx.module_id.as_str()
        );

        // For a stage, "starting" might mean:
        // - Enabling the motor amplifier
        // - Beginning position polling

        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(e) => return RResult::RErr(RString::from(format!("Lock error: {}", e))),
        };

        if !inner.mock_mode {
            let cmd = format!("{}MO\r\n", inner.axis);
            if let Some(port) = &mut inner.port {
                if let Err(e) = port.write_all(cmd.as_bytes()) {
                    return RResult::RErr(RString::from(format!("Serial write error: {}", e)));
                }
            } else {
                warn!(
                    "ESP300: Serial port not open, dropping command: {}",
                    cmd.trim()
                );
            }
        }

        drop(inner);

        self.state = FfiModuleState::Running;
        self.emit_event("started", "Module started", 1);

        RResult::ROk(())
    }

    fn pause(&mut self) -> FfiModuleResult<()> {
        info!("ESP300: Pausing module");

        // Stop any motion in progress
        if let Err(e) = self.stop_motion() {
            warn!("ESP300: Error stopping motion during pause: {}", e);
        }

        self.state = FfiModuleState::Paused;
        self.emit_event("paused", "Module paused", 1);

        RResult::ROk(())
    }

    fn resume(&mut self) -> FfiModuleResult<()> {
        info!("ESP300: Resuming module");

        self.state = FfiModuleState::Running;
        self.emit_event("resumed", "Module resumed", 1);

        RResult::ROk(())
    }

    fn stop(&mut self) -> FfiModuleResult<()> {
        info!("ESP300: Stopping module");

        // Stop any motion
        if let Err(e) = self.stop_motion() {
            error!("ESP300: Error stopping motion: {}", e);
        }

        self.state = FfiModuleState::Stopped;
        self.emit_event("stopped", "Module stopped", 1);

        RResult::ROk(())
    }

    fn poll_event(&mut self) -> ROption<FfiModuleEvent> {
        if let Ok(mut events) = self.events.lock() {
            match events.pop_front() {
                Some(event) => ROption::RSome(event),
                None => ROption::RNone,
            }
        } else {
            ROption::RNone
        }
    }

    fn poll_data(&mut self) -> ROption<FfiModuleDataPoint> {
        if let Ok(mut data) = self.data.lock() {
            match data.pop_front() {
                Some(d) => ROption::RSome(d),
                None => ROption::RNone,
            }
        } else {
            ROption::RNone
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_module() {
        let module = Esp300Stage::new();
        assert_eq!(module.state(), FfiModuleState::Created);
        assert_eq!(module.type_id().as_str(), "esp300_stage");
    }

    #[test]
    fn test_configure() {
        let mut module = Esp300Stage::new();

        let mut params = RHashMap::new();
        params.insert(RString::from("port_path"), RString::from("/dev/ttyUSB1"));
        params.insert(RString::from("axis"), RString::from("2"));
        params.insert(RString::from("velocity"), RString::from("20.0"));
        params.insert(RString::from("mock"), RString::from("true"));

        let result = module.configure(params);
        assert!(result.is_ok());
        assert_eq!(module.state(), FfiModuleState::Configured);
    }

    #[test]
    fn test_invalid_axis() {
        let mut module = Esp300Stage::new();

        let mut params = RHashMap::new();
        params.insert(RString::from("axis"), RString::from("5"));

        let result = module.configure(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_mode_motion() {
        let mut module = Esp300Stage::new();

        // Configure with mock mode
        let mut params = RHashMap::new();
        params.insert(RString::from("mock"), RString::from("true"));
        module.configure(params).unwrap();

        // Stage and start
        let ctx = FfiModuleContext {
            module_id: RString::from("test-esp300"),
            assignments: RHashMap::new(),
            host_context: 0,
        };
        module.stage(&ctx).unwrap();
        module.start(ctx).unwrap();

        // Home first
        module.home().unwrap();

        // Move and check position
        module.move_abs(10.0).unwrap();
        module.wait_settled().unwrap();

        let pos = module.position().unwrap();
        assert!((pos - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_state_serialization() {
        let mut module = Esp300Stage::new();

        // Configure with mock mode
        let mut params = RHashMap::new();
        params.insert(RString::from("mock"), RString::from("true"));
        module.configure(params).unwrap();

        // Home and move
        let ctx = FfiModuleContext {
            module_id: RString::from("test-esp300"),
            assignments: RHashMap::new(),
            host_context: 0,
        };
        module.stage(&ctx).unwrap();
        module.start(ctx).unwrap();
        module.home().unwrap();
        module.move_abs(25.0).unwrap();
        module.wait_settled().unwrap();

        // Serialize state
        let state_json = module.serialize_state().unwrap();
        assert!(state_json.contains("25.0") || state_json.contains("25"));

        // Create new module and restore state
        let new_module = Esp300Stage::new();
        new_module.restore_state(&state_json).unwrap();

        let pos = new_module.position().unwrap();
        assert!((pos - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_type_info() {
        let info = Esp300Stage::type_info_static();
        assert_eq!(info.type_id.as_str(), "esp300_stage");
        assert!(!info.parameters.is_empty());
        assert!(!info.event_types.is_empty());
        assert!(!info.data_types.is_empty());
    }
}
