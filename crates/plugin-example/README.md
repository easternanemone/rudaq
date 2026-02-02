# plugin-example

Example native plugin demonstrating the plugin-api.

## Overview

This crate provides a complete working example of a rust-daq plugin. The plugin demonstrates:

- Module registration and metadata
- Configuration parameter handling
- Complete lifecycle implementation (create → configure → stage → start → stop → unstage)
- Event and data point emission
- FFI-safe trait implementation using `abi_stable`

## The Echo Module

The example plugin exports a simple "echo" module that:

1. Accepts a message configuration parameter
2. Accepts an echo count (1-100)
3. Echoes the message as a data point when started
4. Emits events for start/complete lifecycle

This is intentionally simple to focus on the plugin API mechanics.

## Building

### Build Plugin

```bash
cargo build --release -p plugin-example
```

Output: `target/release/libplugin_example.so` (Linux/macOS) or `.dll` (Windows)

### With Feature Flags

None required - the plugin is self-contained.

## Using the Plugin

### Load with PluginManager

```rust
use plugin_api::PluginManager;

let manager = PluginManager::new();
let plugin = manager.load_plugin("./target/release/libplugin_example.so")?;

println!("Loaded: {}", plugin.metadata().name);
for module_type in &plugin.metadata().module_types {
    println!("  - {}", module_type);
}
```

### Create and Configure Module

```rust
use plugin_api::prelude::*;

let mut module = plugin.create_module("echo_module")?;

// Configure with parameters
let mut config = FfiModuleConfig::new();
config.insert(RString::from("message"), RString::from("Hello Plugin!"));
config.insert(RString::from("echo_count"), RString::from("5"));

let warnings = module.configure(config)?;
for warning in warnings {
    eprintln!("Warning: {}", warning);
}
```

### Run Module

```rust
use plugin_api::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    // Prepare module
    module.stage(&FfiModuleContext::default())?;

    // Run
    let mut ctx = FfiModuleContext::default();
    module.start(ctx)?;

    // Poll events and data
    loop {
        while let Some(event) = module.poll_event() {
            println!("Event: {} - {}", event.event_type, event.message);
        }

        while let Some(data) = module.poll_data() {
            println!("Data: {:?}", data.values);
        }

        if module.state() == FfiModuleState::Stopped {
            break;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Cleanup
    module.stop()?;
    module.unstage(&FfiModuleContext::default())?;
    Ok(())
}
```

## Module Structure

### Entry Point: `get_root_module()`

Exported via `#[abi_stable::export_root_module]`, this is the entry point that `PluginManager` calls when loading the plugin.

Returns a `PluginMod_Ref` containing:
- `abi_version` - Plugin ABI version (must match daemon's version)
- `get_metadata` - Plugin metadata function
- `list_module_types` - Available module type list
- `create_module` - Module factory

### ABI Version Check

The `abi_version_fn` returns `AbiVersion::CURRENT`, which must match the daemon's version. Plugins with mismatched ABI versions are rejected at load time.

### Metadata

The plugin exports metadata about itself:

```rust
PluginMetadata::new("example-plugin", "Example Plugin", "0.1.0")
    .with_author("DAQ Team")
    .with_description("Example plugin demonstrating the plugin API")
```

### Module Factory

The `create_module` function instantiates modules by type ID:

```rust
match type_id.as_str() {
    "echo_module" => {
        let module = EchoModule::new();
        let boxed = ModuleFfi_TO::from_value(module, abi_stable::sabi_trait::TD_CanDowncast);
        RResult::ROk(boxed)
    }
    _ => RResult::RErr(RString::from("Unknown module type")),
}
```

## Module Implementation: EchoModule

### Type Information

Defined in `EchoModule::type_info_static()`, includes:
- Type ID and display name
- Version and description
- Parameter definitions (message, echo_count)
- Event types (echo_started, echo_complete)
- Data types (echo)

### State Machine

```
Created → Configured → Staged → Running → Stopped → Unstaged → Created
```

The module tracks state through the lifecycle:

```rust
fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>> {
    // Parse and validate parameters
    // Update self.message and self.echo_count
    self.state = FfiModuleState::Configured;
    RResult::ROk(RVec::new())  // No warnings
}

fn start(&mut self, _ctx: FfiModuleContext) -> FfiModuleResult<()> {
    self.emit_event("echo_started", 1, "Echo module started");

    for i in 0..self.echo_count {
        self.emit_data(i);  // Generate data points
    }

    self.emit_event("echo_complete", 1, "Echo module completed");
    self.state = FfiModuleState::Running;
    RResult::ROk(())
}
```

### Event Emission

Events are pushed to a queue for polling:

```rust
fn emit_event(&mut self, event_type: &str, severity: u8, message: &str) {
    self.events.push_back(FfiModuleEvent {
        event_type: RString::from(event_type),
        severity,
        message: RString::from(message),
        data: RHashMap::new(),
    });
}
```

### Data Emission

Data points include timestamp and metadata:

```rust
fn emit_data(&mut self, index: u32) {
    let mut values = RHashMap::new();
    values.insert(RString::from("echo_index"), index as f64);
    values.insert(RString::from("message_length"), self.message.len() as f64);

    let mut metadata = RHashMap::new();
    metadata.insert(RString::from("message"), RString::from(self.message.as_str()));

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
```

## Key Design Points

### FFI Safety

All exported functions use `#[abi_stable::sabi_extern_fn]` to ensure stable calling conventions across compiler versions.

### ABI-Stable Types

The module uses `abi_stable` types throughout:
- `RString` instead of `String`
- `RVec<T>` instead of `Vec<T>`
- `RHashMap<K,V>` instead of `HashMap<K,V>`
- `ROption<T>` instead of `Option<T>`

These ensure binary compatibility across different compiler versions.

### Queued Polling

Events and data are queued internally and returned via polling. This avoids callbacks and ensures thread-safe emission:

```rust
fn poll_event(&mut self) -> ROption<FfiModuleEvent> {
    match self.events.pop_front() {
        Some(event) => ROption::RSome(event),
        None => ROption::RNone,
    }
}
```

## Testing

The example can be loaded and tested by the plugin loader:

```bash
# Build
cargo build --release -p plugin-example

# Test in Rust code
use plugin_api::PluginManager;

let manager = PluginManager::new();
let plugin = manager.load_plugin("./target/release/libplugin_example.so")?;
// ... create module, test lifecycle
```

## Extending the Example

To create your own plugin, copy this crate and:

1. Rename `EchoModule` to your module name
2. Add your custom state fields
3. Implement the lifecycle methods
4. Adjust type_info() with your parameters
5. Emit events/data relevant to your module
6. Update metadata

## Related Documentation

- [plugin-api](../plugin-api) - Plugin API reference
- [Module System Design](../../docs/architecture/) - Architecture decisions
- [Scripting Guide](../../docs/guides/rhai-scripting.md) - Alternative to native plugins

## See Also

- `plugin-api` crate - Complete API reference
- `abi_stable` crate - FFI safety documentation
- `plugin-loader` module in `common` crate - Plugin loading implementation
