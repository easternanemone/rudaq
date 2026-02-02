# plugin-api

FFI-stable plugin API for rust-daq modules.

## Overview

The `plugin-api` crate provides the ABI-stable interface for native plugins using the `abi_stable` crate. Plugins implement the `ModuleFfi` trait and export a root module via `get_root_module()`.

Plugins enable extensibility without recompiling the daemon - they are loaded at runtime as shared libraries.

## Architecture

```
PluginManager
├── NativeLoader (abi_stable) ← This crate
├── ScriptLoader (daq-scripting)
└── WasmLoader (future)
```

## Key Types

### ModuleFfi
The core trait that all plugins must implement. Provides the full module lifecycle:

```rust
pub trait ModuleFfi: Send {
    fn type_info(&self) -> FfiModuleTypeInfo;
    fn type_id(&self) -> RString;
    fn state(&self) -> FfiModuleState;
    fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>>;
    fn get_config(&self) -> FfiModuleConfig;
    fn stage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()>;
    fn unstage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()>;
    fn start(&mut self, ctx: FfiModuleContext) -> FfiModuleResult<()>;
    fn pause(&mut self) -> FfiModuleResult<()>;
    fn resume(&mut self) -> FfiModuleResult<()>;
    fn stop(&mut self) -> FfiModuleResult<()>;
    fn poll_event(&mut self) -> ROption<FfiModuleEvent>;
    fn poll_data(&mut self) -> ROption<FfiModuleDataPoint>;
}
```

### FfiModuleState
Lifecycle states for a plugin module:

```
Created → Configured → Staged → Running ↔ Paused → Stopped
                                  ↓
                             (poll_event, poll_data)
```

### FfiModuleTypeInfo
Module metadata and parameter definitions:

```rust
pub struct FfiModuleTypeInfo {
    pub type_id: RString,
    pub display_name: RString,
    pub description: RString,
    pub version: RString,
    pub parameters: RVec<FfiModuleParameter>,
    pub event_types: RVec<RString>,
    pub data_types: RVec<RString>,
    pub required_roles: RVec<RString>,
    pub optional_roles: RVec<RString>,
}
```

### Related Types

- **FfiModuleParameter** - Defines configurable parameters (type, range, defaults)
- **FfiModuleEvent** - Runtime events (started, error, etc.)
- **FfiModuleDataPoint** - Measurement data with timestamp and metadata
- **FfiModuleConfig** - Parameter map for `configure()`

## Creating a Plugin

### 1. Create Plugin Crate

```bash
cargo new --lib my-plugin
cd my-plugin
```

Add to `Cargo.toml`:

```toml
[dependencies]
plugin-api = { path = "../../plugin-api" }
abi_stable = "0.11"

[lib]
crate-type = ["cdylib"]  # Required for .so/.dll
```

### 2. Implement ModuleFfi

```rust
use plugin_api::prelude::*;

pub struct MyModule {
    state: FfiModuleState,
    config: FfiModuleConfig,
}

impl ModuleFfi for MyModule {
    fn type_id(&self) -> RString {
        RString::from("my_module")
    }

    fn state(&self) -> FfiModuleState {
        self.state
    }

    fn type_info(&self) -> FfiModuleTypeInfo {
        // Define your module's metadata
        FfiModuleTypeInfo { ... }
    }

    fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>> {
        self.config = params;
        self.state = FfiModuleState::Configured;
        RResult::ROk(RVec::new())
    }

    fn start(&mut self, _ctx: FfiModuleContext) -> FfiModuleResult<()> {
        self.state = FfiModuleState::Running;
        RResult::ROk(())
    }

    fn stop(&mut self) -> FfiModuleResult<()> {
        self.state = FfiModuleState::Stopped;
        RResult::ROk(())
    }

    // ... implement other methods
}
```

### 3. Export Root Module

```rust
#[abi_stable::export_root_module]
fn get_root_module() -> PluginMod_Ref {
    PluginMod {
        abi_version: abi_version_fn,
        get_metadata: get_metadata_fn,
        list_module_types: list_module_types_fn,
        create_module: create_module_fn,
    }
    .leak_into_prefix()
}

#[abi_stable::sabi_extern_fn]
fn abi_version_fn() -> AbiVersion {
    AbiVersion::CURRENT
}

#[abi_stable::sabi_extern_fn]
fn get_metadata_fn() -> PluginMetadata {
    PluginMetadata::new("my-plugin", "My Plugin", "0.1.0")
        .with_author("Your Name")
        .with_description("Plugin description")
}

#[abi_stable::sabi_extern_fn]
fn list_module_types_fn() -> RVec<FfiModuleTypeInfo> {
    let mut types = RVec::new();
    types.push(MyModule::type_info_static());
    types
}

#[abi_stable::sabi_extern_fn]
fn create_module_fn(type_id: RString) -> RResult<ModuleFfiBox, RString> {
    match type_id.as_str() {
        "my_module" => {
            let module = MyModule::new();
            let boxed = ModuleFfi_TO::from_value(module, TD_CanDowncast);
            RResult::ROk(boxed)
        }
        _ => RResult::RErr(RString::from("Unknown module type")),
    }
}
```

### 4. Build Plugin

```bash
cargo build --release

# Plugin is at: target/release/libmy_plugin.so (Linux/macOS)
# or:          target/release/my_plugin.dll (Windows)
```

## Using abi_stable Types

The plugin API uses `abi_stable`'s stable types for FFI safety:

| Standard Rust | abi_stable | Purpose |
|---------------|-----------|---------|
| `String` | `RString` | Stable string across ABI boundaries |
| `Vec<T>` | `RVec<T>` | Stable vector |
| `HashMap<K,V>` | `RHashMap<K,V>` | Stable map |
| `Option<T>` | `ROption<T>` | Stable optional |
| `Result<T,E>` | `RResult<T,E>` | Stable result |

These types have the same layout across compiler versions and architectures, ensuring plugins remain compatible.

## Prelude

For convenience, use the `prelude`:

```rust
use plugin_api::prelude::*;

// Now available:
// - LoadedPlugin, PluginManager
// - AbiVersion, PluginMetadata
// - ModuleFfi, ModuleFfiBox, FfiModuleState, etc.
// - RHashMap, RString, RVec, etc.
// - abi_stable macros (export_root_module, sabi_extern_fn, etc.)
```

## Module Lifecycle

1. **Created** - Module instantiated, not configured
2. **Configured** - `configure()` called with parameters
3. **Staged** - `stage()` called, resources allocated
4. **Running** - `start()` called, active operation
5. **Paused** - `pause()` called (optional)
6. **Stopped** - `stop()` called, cleanup started
7. **Unstaged** - `unstage()` called, resources freed

## Event and Data Polling

Modules emit events and data points via polling:

```rust
// In main loop
loop {
    // Poll for events
    while let Some(event) = module.poll_event() {
        println!("Event: {}", event.event_type);
    }

    // Poll for data
    while let Some(data) = module.poll_data() {
        println!("Data: {:?}", data.values);
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
}
```

## Parameters

Define configurable parameters in `type_info()`:

```rust
let mut params = RVec::new();
params.push(FfiModuleParameter {
    param_id: RString::from("sample_rate"),
    display_name: RString::from("Sample Rate"),
    description: RString::from("Samples per second"),
    param_type: RString::from("float"),
    default_value: RString::from("1000"),
    min_value: ROption::RSome(RString::from("1")),
    max_value: ROption::RSome(RString::from("100000")),
    enum_values: RVec::new(),
    units: RString::from("Hz"),
    required: true,
});
```

## Example Plugin

See `plugin-example` crate for a complete working example.

```bash
cd ../plugin-example
cargo build --release
```

## Loading Plugins

Plugins are loaded by `PluginManager`:

```rust
use plugin_api::PluginManager;

let manager = PluginManager::new();
let plugin = manager.load_plugin("./target/release/libmy_plugin.so")?;
```

## Related Documentation

- [plugin-example](../plugin-example) - Complete working example
- [Module System Design](../../docs/architecture/adr-module-system.md)
- [Scripting Guide](../../docs/guides/rhai-scripting.md)

## See Also

- `abi_stable` crate documentation - FFI safety details
- `plugin-example` crate - Reference implementation
- `common` crate - Shared data types
