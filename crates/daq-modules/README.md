# daq-modules

Experiment module system for rust-daq, inspired by PyMoDAQ, DynExp, and Bluesky patterns.

## Overview

Modules are reusable experiment components that operate on abstract "roles" rather than specific hardware. This decoupling allows the same module to work with different physical devices.

### Key Concepts

| Concept | Description |
|---------|-------------|
| **Module** | A reusable experiment component (e.g., power monitor, scan controller) |
| **Role** | A capability requirement (e.g., "power_meter" requires `Readable`) |
| **ModuleContext** | Provides device access and event/data emission during execution |
| **ModuleRegistry** | Manages module types and active instances |
| **RunEngine** | Central orchestrator for multi-module experiments |
| **Document** | Bluesky-style self-describing data stream (Start → Descriptor → Event → Stop) |

### Module Lifecycle

Modules follow a Bluesky-inspired lifecycle:

1. **Created** — new instance registered
2. **Configured** — parameters set
3. **Staged** — resources allocated, hardware warmed up
4. **Started** — execution begins
5. **Running** — processing data, emitting events
6. **Paused/Resumed** — optional flow control
7. **Stopped** — execution halted
8. **Unstaged** — resources released (guaranteed, even on error)

## Usage

```rust
use daq_modules::ModuleRegistry;

let mut registry = ModuleRegistry::new(device_registry);

// Create a power monitor instance
let module_id = registry.create_module("power_monitor", "Laser Power")?;

// Assign a real device to the "power_meter" role
registry.assign_device(&module_id, "power_meter", "newport_1830c")?;

// Configure
let mut params = HashMap::new();
params.insert("sample_rate_hz".to_string(), "10.0".to_string());
registry.configure_module(&module_id, params)?;

// Run
registry.stage_module(&module_id).await?;
registry.start_module(&module_id).await?;
```

## Built-in Modules

| Module | Type ID | Required Roles | Description |
|--------|---------|----------------|-------------|
| `PowerMonitor` | `power_monitor` | `power_meter` (Readable) | Continuous power measurement with threshold alerts |

## Creating a Module

1. Implement the `Module` trait:

```rust
use daq_modules::{Module, ModuleContext};

pub struct MyModule { /* state */ }

#[async_trait]
impl Module for MyModule {
    fn type_info() -> ModuleTypeInfo { /* define roles, params */ }
    fn type_id(&self) -> &str { "my_module" }
    async fn start(&mut self, ctx: ModuleContext) -> Result<()> {
        // Spawn background task using ctx
        let reader = ctx.get_readable("sensor")?;
        // ... measurement loop
    }
    // ... other lifecycle methods
}
```

2. Register with the `ModuleRegistry`:

```rust
registry.register_type::<MyModule>();
```

## Plugin System

With the `native_plugins` feature, modules can be loaded from shared libraries at runtime:

```rust
let mut plugin_manager = PluginManager::new();
plugin_manager.add_search_path("./plugins");
plugin_manager.discover_plugins()?;

let count = registry.register_plugin_types(&plugin_manager);
```

## Related

- [`common`](../common/README.md) — Capability traits and observable parameters
- [`hardware`](../hardware/README.md) — Device registry
- [Scripting Guide](../../docs/how-to/scripting.md) — Rhai-based automation (alternative to modules)
