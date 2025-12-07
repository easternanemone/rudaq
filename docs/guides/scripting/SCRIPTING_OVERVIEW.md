# Scripting Engine Overview

## Introduction

The rust-daq V5 architecture includes a powerful scripting engine based on [Rhai](https://rhai.rs/), enabling scientists and engineers to write experiment control logic without recompiling Rust code. This "Headless-First" design separates hardware control (fast, compiled Rust) from experiment logic (flexible, scriptable).

## Why Scripting?

### Traditional Problem
- **Compile-Test Cycle**: Every experiment change required recompiling Rust code
- **Domain Expertise**: Scientists had to learn Rust to change experiment logic
- **Iteration Speed**: Simple parameter changes took minutes instead of seconds

### Scripting Solution
- **Hot-Swappable**: Change experiment logic by editing `.rhai` files
- **No Compilation**: Scripts execute immediately (< 50ms overhead)
- **Familiar Syntax**: Rhai syntax similar to JavaScript/Rust
- **Hardware Safety**: Built-in operation limits prevent runaway scripts

## Architecture

```
┌─────────────────────────────────────────┐
│  Experiment Script (.rhai)              │
│  ┌──────────────────────────────────┐   │
│  │ for i in 0..10 {                 │   │
│  │     stage.move_abs(i * 1.0);     │   │
│  │     camera.trigger();             │   │
│  │ }                                 │   │
│  └──────────────────────────────────┘   │
└────────────┬────────────────────────────┘
             │
             v
┌─────────────────────────────────────────┐
│  ScriptHost (Rhai Engine)               │
│  - Parses and executes Rhai scripts      │
│  - Enforces safety limits (10k ops)     │
│  - Bridges sync→async                    │
└────────────┬────────────────────────────┘
             │
             v
┌─────────────────────────────────────────┐
│  Hardware Bindings (src/scripting/)     │
│  ┌─────────────────────────────────┐    │
│  │  StageHandle → Movable trait    │    │
│  │  CameraHandle → Camera trait    │    │
│  └─────────────────────────────────┘    │
└────────────┬────────────────────────────┘
             │
             v
┌─────────────────────────────────────────┐
│  V5 Capability Traits (async)           │
│  - Movable (stages, actuators)          │
│  - Camera (Triggerable + FrameProducer) │
│  - Readable (power meters, sensors)     │
└────────────┬────────────────────────────┘
             │
             v
┌─────────────────────────────────────────┐
│  Hardware Drivers                        │
│  - Esp300Driver (Newport stage)         │
│  - Ell14Driver (Thorlabs rotator)       │
│  - MockStage, MockCamera (testing)      │
└─────────────────────────────────────────┘
```

## Key Components

### 1. RhaiEngine (`src/scripting/rhai_engine.rs`)
- **Purpose**: V5 scripting backend implementing `ScriptEngine` trait
- **Safety**: Enforces 10,000 operation limit to prevent infinite loops
- **Methods**:
  - `new()` - Create engine with defaults
  - `with_hardware()` - Create with hardware bindings
  - `execute_script(&self, script: &str)` - Async execution
  - `validate_script(&self, script: &str)` - Check syntax

### 2. ScriptHost (`src/scripting/engine.rs`) - DEPRECATED
- **Status**: Legacy V4 wrapper, maintained for backward compatibility.
- **Migration**: Use `RhaiEngine` for all new code.

### 3. Hardware Bindings (`src/scripting/bindings.rs`)
- **Purpose**: Bridges synchronous Rhai to async Rust hardware
- **Pattern**: Uses `tokio::task::block_in_place()` for sync→async conversion
- **Registered Types**:
  - `StageHandle` - Wraps `Arc<dyn Movable>`
  - `CameraHandle` - Wraps `Arc<dyn Camera>`

### 4. CLI Integration (`src/main.rs`)
```bash
# Run script once (for testing)
rust-daq run experiment.rhai

# Run with custom config
rust-daq run --config hardware.toml experiment.rhai

# Start daemon for remote control
rust-daq daemon --port 50051
```

## Safety Features

### Operation Limit
Scripts are limited to **10,000 operations** (configurable):
```rust
// RhaiEngine implementation
engine.on_progress(|count| {
    if count > 10000 {
        Some("Safety limit exceeded".into())
    } else {
        None
    }
});
```

**Why?**: Prevents infinite loops from hanging hardware:
```rhai
// This would terminate after 10,000 iterations
while true {
    stage.move_abs(0.0);  // UNSAFE - will auto-terminate
}
```

### Error Handling
All hardware operations return Results that propagate to scripts:
```rhai
// If move fails, script terminates with error message
stage.move_abs(1000.0);  // Exceeds travel range → script stops
```

## Performance Characteristics

| Metric | Value | Notes |
|--------|-------|-------|
| **Script startup** | < 10ms | Engine initialization |
| **Command overhead** | < 50ms | Per hardware call |
| **Operation limit** | 10,000 | Configurable safety |
| **Memory overhead** | ~5MB | Rhai engine |

### When to Use Scripting
- **Experiment sequences** (scans, acquisitions)
- **Parameter sweeps** (changing voltages, positions)
- **Conditional logic** (if temp > 50, stop)
- **Repeated workflows** (daily calibrations)

### When NOT to Use Scripting
- **High-frequency acquisition** (> 1kHz) - Use compiled Rust
- **Real-time control** - Use compiled Rust
- **Complex analysis** - Use Python client with gRPC
- **Safety-critical** - Use compiled Rust with testing

## Async→Sync Bridge Pattern

### The Problem
Rust hardware drivers are **async** (non-blocking):
```rust
async fn move_abs(&self, position: f64) -> Result<()>;
```

Rhai scripts are **synchronous** (blocking):
```rhai
stage.move_abs(10.0);  // Must wait for completion
```

### The Solution
`tokio::task::block_in_place()` bridges the gap:
```rust
engine.register_fn("move_abs", move |stage: &mut StageHandle, pos: f64| {
    block_in_place(|| {
        Handle::current().block_on(stage.driver.move_abs(pos))
    }).unwrap()
});
```

**What this does**:
1. `block_in_place()` - Tells Tokio "this thread will block"
2. `block_on()` - Converts async call to blocking
3. `.unwrap()` - Propagates errors to script

**Why it's safe**:
- Tokio spawns extra worker threads if needed
- Script thread doesn't block async runtime
- Hardware still runs asynchronously

## Comparison with Other Approaches

| Approach | Pros | Cons | Use Case |
|----------|------|------|----------|
| **Rhai Scripts** | Fast to write, no compilation | Limited language features | Experiment control |
| **Python (gRPC)** | Full ecosystem, data analysis | Network overhead (~1ms) | Analysis, visualization |
| **Compiled Rust** | Maximum performance | Slow iteration | Core drivers, RT control |

## Integration with V5 Architecture

### Capability Traits
Scripts interact with hardware through V5 capability traits:
```rhai
// Movable trait
stage.move_abs(10.0);
stage.wait_settled();

// Camera trait (Triggerable + FrameProducer)
camera.arm();
camera.trigger();
```

### Headless-First Design
Scripts run on the **headless server**, not the GUI client:
```
┌─────────────┐                 ┌──────────────┐
│ GUI Client  │  ←── gRPC ──→   │ Headless     │
│ (Remote)    │                 │ Server       │
│             │                 │              │
│ - Display   │                 │ - Scripts    │
│ - Controls  │                 │ - Hardware   │
└─────────────┘                 │ - Data       │
                                 └──────────────┘
```

## Next Steps

- **[Rhai API Reference](./RHAI_API_REFERENCE.md)** - Complete function documentation
- **[Scripting Examples](./SCRIPTING_EXAMPLES.md)** - Common patterns and recipes
- **[Async Bridge Guide](./ASYNC_BRIDGE_GUIDE.md)** - Deep dive into sync→async conversion

## See Also

- [V5 Architecture](../architecture/V5_ARCHITECTURE.md)
- [Hardware Capability Traits](../../src/hardware/capabilities.rs)
- [Rhai Language Reference](https://rhai.rs/book/)
