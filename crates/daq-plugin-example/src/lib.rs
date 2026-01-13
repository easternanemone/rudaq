//! Example plugin demonstrating the daq-plugin-api.
//!
//! This plugin provides a simple "echo" module that demonstrates the full
//! plugin lifecycle and FFI-safe trait implementation.

use daq_plugin_api::prelude::*;
use std::collections::VecDeque;

// =============================================================================
// Plugin Entry Point
// =============================================================================

/// Export the plugin root module.
///
/// This is the entry point that the PluginManager calls when loading the plugin.
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

#[abi_stable::sabi_extern_fn]
fn abi_version() -> AbiVersion {
    AbiVersion::CURRENT
}

#[abi_stable::sabi_extern_fn]
fn get_metadata() -> PluginMetadata {
    PluginMetadata::new("example-plugin", "Example Plugin", "0.1.0")
        .with_author("DAQ Team")
        .with_description("Example plugin demonstrating the plugin API")
        .with_module_type("echo_module")
}

#[abi_stable::sabi_extern_fn]
fn list_module_types() -> RVec<FfiModuleTypeInfo> {
    let mut types = RVec::new();
    types.push(EchoModule::type_info_static());
    types
}

#[abi_stable::sabi_extern_fn]
fn create_module(type_id: RString) -> RResult<ModuleFfiBox, RString> {
    match type_id.as_str() {
        "echo_module" => {
            let module = EchoModule::new();
            // Convert to trait object using abi_stable's mechanism
            let boxed = ModuleFfi_TO::from_value(module, abi_stable::sabi_trait::TD_CanDowncast);
            RResult::ROk(boxed)
        }
        _ => RResult::RErr(RString::from(format!("Unknown module type: {}", type_id))),
    }
}

// =============================================================================
// Echo Module Implementation
// =============================================================================

/// A simple echo module that demonstrates the plugin API.
///
/// This module:
/// - Accepts a "message" configuration parameter
/// - Echoes the message as a data point when started
/// - Demonstrates the full lifecycle (configure -> stage -> start -> stop -> unstage)
struct EchoModule {
    state: FfiModuleState,
    message: String,
    echo_count: u32,
    events: VecDeque<FfiModuleEvent>,
    data: VecDeque<FfiModuleDataPoint>,
}

impl EchoModule {
    fn new() -> Self {
        Self {
            state: FfiModuleState::Created,
            message: "Hello from plugin!".to_string(),
            echo_count: 3,
            events: VecDeque::new(),
            data: VecDeque::new(),
        }
    }

    fn type_info_static() -> FfiModuleTypeInfo {
        FfiModuleTypeInfo {
            type_id: RString::from("echo_module"),
            display_name: RString::from("Echo Module"),
            description: RString::from("A simple module that echoes a configured message"),
            version: RString::from("0.1.0"),
            parameters: {
                let mut params = RVec::new();
                params.push(FfiModuleParameter {
                    param_id: RString::from("message"),
                    display_name: RString::from("Message"),
                    description: RString::from("The message to echo"),
                    param_type: RString::from("string"),
                    default_value: RString::from("Hello from plugin!"),
                    min_value: ROption::RNone,
                    max_value: ROption::RNone,
                    enum_values: RVec::new(),
                    units: RString::new(),
                    required: false,
                });
                params.push(FfiModuleParameter {
                    param_id: RString::from("echo_count"),
                    display_name: RString::from("Echo Count"),
                    description: RString::from("Number of times to echo the message"),
                    param_type: RString::from("integer"),
                    default_value: RString::from("3"),
                    min_value: ROption::RSome(RString::from("1")),
                    max_value: ROption::RSome(RString::from("100")),
                    enum_values: RVec::new(),
                    units: RString::new(),
                    required: false,
                });
                params
            },
            event_types: {
                let mut types = RVec::new();
                types.push(RString::from("echo_started"));
                types.push(RString::from("echo_complete"));
                types
            },
            data_types: {
                let mut types = RVec::new();
                types.push(RString::from("echo"));
                types
            },
            required_roles: RVec::new(),
            optional_roles: RVec::new(),
        }
    }

    fn emit_event(&mut self, event_type: &str, severity: u8, message: &str) {
        self.events.push_back(FfiModuleEvent {
            event_type: RString::from(event_type),
            severity,
            message: RString::from(message),
            data: RHashMap::new(),
        });
    }

    fn emit_data(&mut self, index: u32) {
        let mut values = RHashMap::new();
        values.insert(RString::from("echo_index"), index as f64);
        values.insert(RString::from("message_length"), self.message.len() as f64);

        let mut metadata = RHashMap::new();
        metadata.insert(
            RString::from("message"),
            RString::from(self.message.as_str()),
        );

        self.data.push_back(FfiModuleDataPoint {
            data_type: RString::from("echo"),
            timestamp_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
            values,
            metadata,
        });
    }
}

impl ModuleFfi for EchoModule {
    fn type_info(&self) -> FfiModuleTypeInfo {
        Self::type_info_static()
    }

    fn type_id(&self) -> RString {
        RString::from("echo_module")
    }

    fn state(&self) -> FfiModuleState {
        self.state
    }

    fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>> {
        let mut warnings = RVec::new();

        if let Some(message) = params.get(&RString::from("message")) {
            self.message = message.to_string();
        }

        if let Some(count) = params.get(&RString::from("echo_count")) {
            match count.parse::<u32>() {
                Ok(n) if (1..=100).contains(&n) => self.echo_count = n,
                Ok(n) => {
                    warnings.push(RString::from(format!(
                        "echo_count {} out of range, using default",
                        n
                    )));
                }
                Err(_) => {
                    warnings.push(RString::from(format!(
                        "Invalid echo_count '{}', using default",
                        count
                    )));
                }
            }
        }

        self.state = FfiModuleState::Configured;
        RResult::ROk(warnings)
    }

    fn get_config(&self) -> FfiModuleConfig {
        let mut config = RHashMap::new();
        config.insert(
            RString::from("message"),
            RString::from(self.message.as_str()),
        );
        config.insert(
            RString::from("echo_count"),
            RString::from(self.echo_count.to_string()),
        );
        config
    }

    fn stage(&mut self, _ctx: &FfiModuleContext) -> FfiModuleResult<()> {
        self.events.clear();
        self.data.clear();
        self.state = FfiModuleState::Staged;
        RResult::ROk(())
    }

    fn unstage(&mut self, _ctx: &FfiModuleContext) -> FfiModuleResult<()> {
        self.state = FfiModuleState::Created;
        RResult::ROk(())
    }

    fn start(&mut self, _ctx: FfiModuleContext) -> FfiModuleResult<()> {
        self.emit_event("echo_started", 1, "Echo module started");

        // Emit configured number of echo data points
        for i in 0..self.echo_count {
            self.emit_data(i);
        }

        self.emit_event("echo_complete", 1, "Echo module completed");
        self.state = FfiModuleState::Running;
        RResult::ROk(())
    }

    fn pause(&mut self) -> FfiModuleResult<()> {
        self.state = FfiModuleState::Paused;
        RResult::ROk(())
    }

    fn resume(&mut self) -> FfiModuleResult<()> {
        self.state = FfiModuleState::Running;
        RResult::ROk(())
    }

    fn stop(&mut self) -> FfiModuleResult<()> {
        self.state = FfiModuleState::Stopped;
        RResult::ROk(())
    }

    fn poll_event(&mut self) -> ROption<FfiModuleEvent> {
        match self.events.pop_front() {
            Some(event) => ROption::RSome(event),
            None => ROption::RNone,
        }
    }

    fn poll_data(&mut self) -> ROption<FfiModuleDataPoint> {
        match self.data.pop_front() {
            Some(data) => ROption::RSome(data),
            None => ROption::RNone,
        }
    }
}
