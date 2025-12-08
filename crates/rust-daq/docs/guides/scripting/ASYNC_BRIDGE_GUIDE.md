# Async Bridge Guide: How Rhai Calls Async Rust

Deep dive into the synchronous-to-asynchronous bridge that enables Rhai scripts to control async Rust hardware drivers.

## The Fundamental Problem

### Async Rust Hardware
Hardware drivers in rust-daq use async/await for non-blocking I/O:

```rust
#[async_trait]
pub trait Movable: Send + Sync {
    async fn move_abs(&self, position: f64) -> Result<()>;
    async fn position(&self) -> Result<f64>;
    async fn wait_settled(&self) -> Result<()>;
}
```

**Why async?**
- Non-blocking: Stage can move while camera captures
- Efficient: Tokio runtime multiplexes I/O operations
- Scalable: Hundreds of devices on single thread

### Synchronous Rhai Scripts
Rhai is a synchronous scripting language:

```rhai
stage.move_abs(10.0);  // Must block until complete
let pos = stage.position();  // Must return immediately
print(`Position: ${pos}mm`);
```

**Why synchronous?**
- Simple mental model: Sequential execution
- No async/await keywords to learn
- Familiar to scientists (Python-like)

### The Mismatch
```
Rhai (sync)          Rust (async)
-----------          ------------
move_abs() ────X───→ async fn move_abs()
                     (Can't directly call!)
```

## The Solution: Tokio's block_in_place

### What is block_in_place?

`tokio::task::block_in_place()` tells the Tokio runtime:
> "This thread is about to block. Spawn another worker thread if needed."

This prevents blocking operations from starving the async runtime.

### The Bridge Pattern

```rust
use tokio::runtime::Handle;
use tokio::task::block_in_place;

engine.register_fn("move_abs", move |stage: &mut StageHandle, pos: f64| {
    // 1. Enter blocking context
    block_in_place(|| {
        // 2. Get current async runtime
        let handle = Handle::current();

        // 3. Block on async function
        handle.block_on(stage.driver.move_abs(pos))
    }).unwrap()  // 4. Propagate errors to Rhai
});
```

### Step-by-Step Breakdown

**Step 1: Enter Blocking Context**
```rust
block_in_place(|| {
    // This closure will block the current thread
    // Tokio spawns extra workers if needed
})
```

**Step 2: Get Current Runtime**
```rust
let handle = Handle::current();
```
- Gets handle to the Tokio runtime
- Allows blocking code to use async runtime

**Step 3: Block on Async Function**
```rust
handle.block_on(stage.driver.move_abs(pos))
```
- Converts `async fn` into blocking call
- Script thread waits for completion
- Other async tasks continue running

**Step 4: Error Propagation**
```rust
.unwrap()
```
- Converts `Result<T, E>` to `T` or panic
- Panic terminates Rhai script with error message

## Complete Example

### Rust Side: Hardware Bindings

```rust
// src/scripting/bindings.rs

use rhai::Engine;
use tokio::runtime::Handle;
use tokio::task::block_in_place;
use std::sync::Arc;

#[derive(Clone)]
pub struct StageHandle {
    pub driver: Arc<dyn Movable>,
}

pub fn register_hardware(engine: &mut Engine) {
    // Register Stage type
    engine.register_type_with_name::<StageHandle>("Stage");

    // Register move_abs method
    engine.register_fn("move_abs", move |stage: &mut StageHandle, pos: f64| {
        block_in_place(|| {
            Handle::current().block_on(stage.driver.move_abs(pos))
        }).unwrap()
    });

    // Register position method (returns value)
    engine.register_fn("position", move |stage: &mut StageHandle| -> f64 {
        block_in_place(|| {
            Handle::current().block_on(stage.driver.position())
        }).unwrap()
    });

    // Register wait_settled method
    engine.register_fn("wait_settled", move |stage: &mut StageHandle| {
        block_in_place(|| {
            Handle::current().block_on(stage.driver.wait_settled())
        }).unwrap()
    });
}
```

### Rhai Side: Script Usage

```rhai
// Script executes synchronously
print("Moving to 10mm...");
stage.move_abs(10.0);  // Blocks until command sent

print("Waiting for motion...");
stage.wait_settled();  // Blocks until motion complete

print("Reading position...");
let pos = stage.position();  // Blocks, returns value
print(`Final position: ${pos}mm`);
```

## Execution Flow Diagram

```
Rhai Script Thread          Tokio Runtime
------------------          -------------

stage.move_abs(10.0)
        |
        v
block_in_place() ────────→ Spawn extra worker?
        |                         |
        v                         v
Handle::current()           (if needed)
        |
        v
block_on(async fn) ──────→ Execute async task
        |                         |
     (BLOCKS)                     |
        |                    Poll hardware
        |                         |
        |                    await ready
        |                         |
        ←─────────────────── Complete
        |
     Returns
        |
        v
print("Done")
```

## Why This Works

### 1. Thread Safety
```rust
#[derive(Clone)]
pub struct StageHandle {
    pub driver: Arc<dyn Movable>,  // Arc = thread-safe reference counting
}
```
- `Arc<T>` allows multiple Rhai closures to share hardware
- Interior mutability (`Mutex`) in driver handles concurrent access

### 2. Runtime Integration
```rust
Handle::current()
```
- Gets handle to existing Tokio runtime (started in `main.rs`)
- Script thread participates in runtime without being async

### 3. Cooperative Blocking
```rust
block_in_place(|| { ... })
```
- Tells scheduler: "I'm blocking, don't wait for me"
- Other async tasks continue running
- Prevents deadlocks

## Performance Characteristics

### Overhead Breakdown

| Component | Time | Notes |
|-----------|------|-------|
| Rhai function call | ~1µs | Minimal |
| `block_in_place()` setup | ~10µs | One-time |
| `block_on()` overhead | ~5µs | Per call |
| Actual hardware operation | ~50ms | Dominant |

**Total script overhead:** ~50ms per hardware call (dominated by hardware, not bridge)

### When Overhead Matters

**Low-frequency** (< 100Hz): Overhead negligible
```rhai
for i in 0..10 {
    stage.move_abs(i * 1.0);  // ~50ms each
}
// Total: ~500ms, bridge adds < 1ms
```

**High-frequency** (> 1kHz): Overhead significant
```rhai
for i in 0..10000 {
    let pos = stage.position();  // 10µs bridge + 1ms hardware
}
// Total: ~10s, bridge adds ~100ms (1%)
```

## Common Pitfalls

### Pitfall 1: Nested Blocking

**WRONG:**
```rust
engine.register_fn("bad_fn", || {
    block_in_place(|| {
        block_in_place(|| {  // Nested!
            // ...
        })
    })
});
```

**Why it fails:** Double blocking can deadlock

**CORRECT:**
```rust
engine.register_fn("good_fn", || {
    block_in_place(|| {
        // Single blocking context
    })
});
```

### Pitfall 2: Forgetting block_in_place

**WRONG:**
```rust
engine.register_fn("move_abs", |stage: &mut StageHandle, pos: f64| {
    Handle::current().block_on(stage.driver.move_abs(pos))  // No block_in_place!
});
```

**Why it fails:** Blocks Tokio worker thread, starves runtime

**CORRECT:**
```rust
engine.register_fn("move_abs", |stage: &mut StageHandle, pos: f64| {
    block_in_place(|| {
        Handle::current().block_on(stage.driver.move_abs(pos))
    })
});
```

### Pitfall 3: Panicking on Errors

**WRONG:**
```rust
.unwrap()  // Panics terminate script immediately
```

**Better:**
```rust
.map_err(|e| format!("Hardware error: {}", e))
.unwrap_or_else(|e| {
    eprintln!("{}", e);
    // Return default or continue
})
```

## Advanced: Custom Error Handling

### Graceful Error Propagation

```rust
use rhai::EvalAltResult;

engine.register_fn("safe_move", |stage: &mut StageHandle, pos: f64| -> Result<(), Box<EvalAltResult>> {
    block_in_place(|| {
        Handle::current().block_on(stage.driver.move_abs(pos))
    })
    .map_err(|e| format!("Move failed: {}", e).into())
});
```

Rhai script:
```rhai
// Script can handle error
try {
    stage.safe_move(1000.0);  // Out of range
} catch(err) {
    print(`Error: ${err}`);
    print("Continuing anyway...");
}
```

### Timeout Handling

```rust
use tokio::time::{timeout, Duration};

engine.register_fn("move_abs_timeout", |stage: &mut StageHandle, pos: f64| {
    block_in_place(|| {
        Handle::current().block_on(async {
            timeout(
                Duration::from_secs(30),
                stage.driver.move_abs(pos)
            ).await
        })
    })
    .map_err(|_| "Timeout")
    .and_then(|r| r.map_err(|e| e.to_string()))
    .unwrap()
});
```

## Comparison with Alternatives

### Alternative 1: Fully Async Scripts (Rejected)

**Hypothetical:**
```rhai
// If Rhai supported async (it doesn't)
async fn main() {
    await stage.move_abs(10.0);
    await stage.wait_settled();
}
```

**Why rejected:**
- Rhai doesn't support async
- Would require custom async runtime for scripts
- More complex for users

### Alternative 2: Message Passing (Rejected)

**Hypothetical:**
```rhai
// Script sends message, continues
send_command("move_abs", 10.0);
send_command("wait_settled");
```

**Why rejected:**
- No feedback to script
- Can't return values (`position()`)
- Error handling unclear

### Alternative 3: Callbacks (Rejected)

**Hypothetical:**
```rhai
stage.move_abs(10.0, |result| {
    print("Move complete");
});
```

**Why rejected:**
- Callback hell for complex sequences
- Difficult error handling
- Non-sequential execution confusing

## Testing the Bridge

### Unit Test Example

```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_async_bridge() {
    // Create Rhai engine with bindings
    let mut engine = Engine::new();
    register_hardware(&mut engine);

    // Create mock hardware
    let stage = Arc::new(MockStage::new());

    // Create scope with hardware
    let mut scope = Scope::new();
    scope.push("stage", StageHandle { driver: stage.clone() });

    // Execute script
    let script = r#"
        stage.move_abs(10.0);
        stage.wait_settled();
        let pos = stage.position();
        pos
    "#;

    let result: f64 = engine.eval_with_scope(&mut scope, script).unwrap();
    assert_eq!(result, 10.0);

    // Verify hardware was called
    assert_eq!(stage.position().await.unwrap(), 10.0);
}
```

## See Also

- [Rhai API Reference](./RHAI_API_REFERENCE.md) - Functions exposed to scripts
- [Scripting Overview](./SCRIPTING_OVERVIEW.md) - High-level architecture
- [Tokio Documentation](https://tokio.rs/tokio/topics/bridging) - Official bridging guide
- [Rhai Book](https://rhai.rs/book/rust/functions.html) - Custom function registration
