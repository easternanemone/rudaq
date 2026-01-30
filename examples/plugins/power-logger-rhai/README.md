# Power Logger - Rhai Script Plugin Example

This example demonstrates how to create a **script-based module plugin** using the Rhai scripting language. Script plugins offer rapid development and hot-reload capabilities without requiring Rust compilation.

## Overview

The Power Logger module continuously reads power values from a device and logs them with timestamps. It demonstrates:

- **Module lifecycle**: `stage()` -> `start()` -> `pause()`/`resume()` -> `stop()` -> `unstage()`
- **Role-based device binding**: Requires a device with `readable` capability
- **Parameter configuration**: Configurable log interval, format, and output path
- **Data emission**: Structured data for the storage layer
- **Hot-reload**: Modify the script while running without recompilation

## Plugin Structure

```
power-logger-rhai/
├── plugin.toml          # Plugin manifest (discovery & metadata)
├── power_logger.rhai    # Module implementation in Rhai
└── README.md            # This file
```

## Quick Start

### 1. Discovery

The plugin is discovered automatically when you add its parent directory to the plugin search path:

```rust
use daq_hardware::plugin::PluginRegistry;

let mut registry = PluginRegistry::new();
registry.add_search_path("./examples/plugins/");
let errors = registry.scan();

// List discovered plugins
for info in registry.list() {
    println!("{} v{} ({})",
        info.name(),
        info.version,
        info.plugin_type()
    );
}
```

### 2. Instantiation

Create a module instance from the discovered plugin:

```rust
// Get the plugin info
let plugin = registry.get_latest("power-logger-rhai")
    .expect("Plugin not found");

// Create module instance (handled by script loader)
let loader = ScriptPluginLoader::new();
let module = loader.create_module(&plugin).await?;
```

### 3. Configuration

Configure the module before staging:

```rust
let params = HashMap::from([
    ("log_interval_ms".to_string(), "500".to_string()),
    ("log_format".to_string(), "csv".to_string()),
    ("output_path".to_string(), "/data/power.csv".to_string()),
]);

let warnings = module.configure(params)?;
for warning in warnings {
    eprintln!("Warning: {}", warning);
}
```

### 4. Device Binding

Bind devices to required roles:

```rust
// Get a power meter device from the hardware registry
let power_meter = hardware_registry.get_device("PM100D-1")?;

// Bind to the power_source role
module.bind_device("power_source", power_meter)?;
```

### 5. Lifecycle

Run the module through its lifecycle:

```rust
// Prepare resources
module.stage()?;

// Begin logging
module.start()?;

// ... experiment runs ...

// Optionally pause/resume
module.pause()?;
tokio::time::sleep(Duration::from_secs(5)).await;
module.resume()?;

// Stop and cleanup
module.stop()?;
module.unstage()?;
```

## Script Module Interface

Script modules implement the following functions:

| Function | Required | Description |
|----------|----------|-------------|
| `module_type_info()` | Yes | Returns module metadata |
| `configure(params)` | Yes | Applies configuration parameters |
| `get_config()` | Yes | Returns current configuration |
| `stage(ctx)` | Yes | Prepares resources and validates bindings |
| `start(ctx)` | Yes | Begins module execution |
| `pause()` | No | Temporarily suspends execution |
| `resume()` | No | Continues after pause |
| `stop()` | Yes | Ends execution |
| `unstage(ctx)` | Yes | Releases all resources |

### Context Object

The `ctx` parameter passed to lifecycle functions contains:

```javascript
ctx = #{
    module_id: "power_logger_001",  // Unique instance ID
    devices: #{                      // Bound devices by role
        power_source: <device_handle>
    }
}
```

## Parameters

| Parameter | Type | Default | Range | Description |
|-----------|------|---------|-------|-------------|
| `log_interval_ms` | int | 1000 | 100-60000 | Milliseconds between readings |
| `log_format` | enum | "simple" | simple/verbose/csv | Output format |
| `output_path` | string | /tmp/power_log.csv | - | CSV output file path |

## Hot-Reload

One of the key advantages of script plugins is hot-reload support:

1. **Start the module** as normal
2. **Edit the script** (`power_logger.rhai`)
3. **Save the file**
4. The runtime detects the change and reloads automatically
5. Module state is preserved, function implementations update

This enables rapid iteration during experiment development without stopping data acquisition.

## Comparison: Script vs Native Plugins

| Aspect | Script Plugin | Native Plugin |
|--------|--------------|---------------|
| Language | Rhai | Rust |
| Compilation | None | Required |
| Hot-reload | Yes | No (requires recompile) |
| Performance | Good | Best |
| Type safety | Runtime | Compile-time |
| Best for | Prototyping, experiment scripts | Production, performance-critical |

## Use Cases

Script plugins are ideal for:

- **Experiment workflows**: Quick iteration on data acquisition logic
- **Custom logging**: Application-specific data formatting
- **Prototyping**: Test ideas before native implementation
- **One-off scripts**: Tasks that don't need optimization

## Events Emitted

| Event Type | Description | Data |
|------------|-------------|------|
| `log_started` | Logging has begun | interval_ms, format, output_path |
| `log_stopped` | Logging has ended | - |
| `power_reading` | New reading available | timestamp, power_mw |
| `error` | An error occurred | message |

## Data Types

### `power_log`

```json
{
    "timestamp": 1705420800000,
    "module_id": "power_logger_001",
    "power_mw": 42.5,
    "energy_mwh": 0.0118,
    "log_number": 42
}
```

## See Also

- `hardware/src/plugin/manifest.rs` - Plugin manifest schema
- `hardware/src/plugin/discovery.rs` - Plugin discovery system
- `examples/scripts/` - Additional Rhai script examples
- `crates/plugin-example/` - Native plugin example
