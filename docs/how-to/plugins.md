# rust-daq Plugin System

The plugin system allows extending rust-daq with new hardware drivers and modules without modifying the core codebase.

## Plugin Types

| Type | Effort | Use Case |
|------|--------|----------|
| **Config-only** | ~30 min | Simple ASCII/SCPI instruments |
| **Native Rust** | 2-4 hours | Complex state machines, high-performance |
| **Rhai Script** | ~1 hour | Experiment workflows, custom modules |

## Quick Comparison

```
┌─────────────────┬────────────────────┬──────────────────┬──────────────────┐
│                 │ Config-Only        │ Native Rust      │ Rhai Script      │
├─────────────────┼────────────────────┼──────────────────┼──────────────────┤
│ Language        │ TOML               │ Rust             │ Rhai             │
│ Compilation     │ None               │ Required         │ None             │
│ Hot-reload      │ Yes                │ Partial          │ Yes              │
│ Performance     │ Good               │ Excellent        │ Fair             │
│ Complexity      │ Simple protocols   │ Any              │ Moderate         │
│ Dependencies    │ None               │ abi_stable       │ None             │
└─────────────────┴────────────────────┴──────────────────┴──────────────────┘
```

## Directory Structure

Plugins live in the `~/.rust-daq/plugins/` directory:

```
~/.rust-daq/plugins/
├── ell14-config/           # Config-only plugin
│   ├── plugin.toml
│   └── device.toml
├── esp300-native/          # Native plugin
│   ├── plugin.toml
│   └── libesp300_native.dylib
└── power-logger-rhai/      # Script plugin
    ├── plugin.toml
    └── power_logger.rhai
```

## Plugin Manifest (plugin.toml)

Every plugin requires a `plugin.toml` manifest:

```toml
[plugin]
id = "my-plugin"                    # Unique identifier
name = "My Plugin"                  # Display name
version = "1.0.0"                   # Semver version
author = "Your Name"
description = "What this plugin does"
min_daq_version = "0.5.0"           # Minimum rust-daq version
type = "config"                     # config, native, or script
```

## Config-Only Plugins

The simplest plugin type - just TOML files defining a device protocol.

**When to use:**
- Device uses ASCII command/response protocol
- No complex state machine needed
- Standard serial or TCP connection

**Structure:**
```
my-device/
├── plugin.toml     # Manifest
└── device.toml     # Protocol definition
```

**Example plugin.toml:**
```toml
[plugin]
id = "my-device"
name = "My Device Driver"
version = "1.0.0"
type = "config"

[[devices]]
type_id = "my_device"
display_name = "My Device"
config_file = "device.toml"
```

**See:** `examples/plugins/ell14-config/` for a complete example.

## Native Rust Plugins

Compiled Rust code loaded as a dynamic library (cdylib).

**When to use:**
- Complex state machine logic
- High-performance I/O requirements
- Proprietary or unusual protocols
- Need to use external Rust crates

**Structure:**
```
esp300-native/
├── plugin.toml     # Manifest
├── Cargo.toml      # Rust project config
└── src/
    └── lib.rs      # Plugin implementation
```

**Key implementation steps:**

1. Add dependencies:
```toml
[dependencies]
abi_stable = "0.11"
plugin-api = { path = "path/to/plugin-api" }
```

2. Export root module:
```rust
use plugin_api::prelude::*;

#[export_root_module]
fn get_root_module() -> PluginMod_Ref {
    PluginMod {
        abi_version,
        get_metadata,
        list_module_types,
        create_module,
    }.leak_into_prefix()
}
```

3. Implement ModuleFfi trait:
```rust
impl ModuleFfi for MyModule {
    fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>>;
    fn stage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()>;
    fn start(&mut self, ctx: FfiModuleContext) -> FfiModuleResult<()>;
    fn stop(&mut self) -> FfiModuleResult<()>;
    fn unstage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()>;
    // ... and more
}
```

**See:** `examples/plugins/esp300-native/` for a complete example.

## Rhai Script Plugins

Scripted modules using the Rhai language - no compilation needed.

**When to use:**
- Rapid iteration / hot-reload
- Experiment workflows
- User-customizable logic
- Moderate performance requirements

**Structure:**
```
power-logger-rhai/
├── plugin.toml         # Manifest with module definitions
└── power_logger.rhai   # Module implementation
```

**Script structure:**
```javascript
// State variables
let interval_ms = 1000;
let is_running = false;

// Lifecycle functions
fn configure(params) { ... }
fn stage(context) { ... }
fn start(context) { ... }
fn stop() { ... }
fn unstage(context) { ... }

// Polling functions
fn poll_event() { return #{ event_type: "...", ... } or (); }
fn poll_data() { return #{ data_type: "...", ... } or (); }
```

**See:** `examples/plugins/power-logger-rhai/` for a complete example.

## Plugin Discovery

Plugins are discovered from:
1. `~/.rust-daq/plugins/` (user plugins)
2. `/etc/rust-daq/plugins/` (system plugins)
3. Paths in `RUST_DAQ_PLUGIN_PATH` environment variable

## Using Plugins

Reference plugins in experiment configurations:

```toml
# Config-only device
[[devices]]
name = "rotator"
type = "ell14-config.ell14"
port = "/dev/ttyUSB0"

# Native device
[[devices]]
name = "stage"
type = "esp300-native.esp300"
port = "/dev/ttyUSB1"

# Script module
[[modules]]
name = "logger"
type = "power-logger-rhai.power_logger"

[modules.parameters]
interval_ms = "500"

[modules.roles]
detector = "power_meter"
```

## Hot-Reload

Config and script plugins support hot-reload:
- Edit the TOML or Rhai file
- Changes take effect on next device/module instantiation
- No daemon restart required

Native plugins have partial hot-reload:
- Library is reloaded on explicit reload command
- Existing instances continue with old code
- New instances use updated code

## Debugging Plugins

Enable debug logging:
```bash
RUST_LOG=plugin_api=debug rust-daq-daemon
```

Common issues:
- **Plugin not found**: Check `plugin.toml` exists and `id` is unique
- **ABI mismatch**: Rebuild native plugin against current plugin-api
- **Script error**: Check Rhai syntax and variable scope

## Example Plugins

See `examples/plugins/` for working examples:
- `ell14-config/` - Config-only (simplest)
- `esp300-native/` - Native Rust (most capable)
- `power-logger-rhai/` - Rhai script (easiest iteration)
