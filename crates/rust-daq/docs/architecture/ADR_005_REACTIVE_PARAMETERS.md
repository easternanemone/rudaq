# ADR 005: Async Hardware Callbacks for Reactive Parameters

**Status:** Accepted (2025-12-02)

**Context:**

The V5 reactive parameter system initially used synchronous callbacks for hardware writes:

```rust
type HardwareWriter<T> = Arc<dyn Fn(T) -> Result<()> + Send + Sync>;
```

This created a fundamental incompatibility with rust-daq's async hardware drivers (tokio_serial, network drivers, etc.). Hardware operations like serial port writes, network requests, and device communication are inherently async in the tokio ecosystem.

**Problem:**

When `Parameter::set()` was called, it needed to trigger hardware writes. With synchronous callbacks, drivers had three bad options:

1. **spawn_blocking**: Blocks threadpool threads, dangerous in async context, breaks error propagation
2. **tokio::spawn**: Spawns detached tasks, breaks error propagation, makes debugging nightmarish
3. **block_on**: Deadlocks if called from within tokio runtime

All three approaches violated Rust async best practices and led to "Split Brain" where `Parameter.set()` updated software state but hardware didn't move reliably.

**Decision:**

Refactor `Parameter<T>` to use async hardware callbacks:

```rust
type AsyncWriter<T> = Arc<dyn Fn(T) -> BoxFuture<'static, Result<()>> + Send + Sync>;
```

**Implementation:**

Since `Parameter::set()` is already async, it can transparently await the BoxFuture:

```rust
impl<T: Clone + Send + Sync + 'static> Parameter<T> {
    pub async fn set(&self, value: T) -> Result<()> {
        // Validate
        self.inner.validate(&value)?;

        // Write to hardware (async)
        if let Some(writer) = &self.hardware_writer {
            writer(value.clone()).await?;
        }

        // Update observable (broadcasts to subscribers)
        self.inner.set(value.clone())?;

        // Notify change listeners
        let listeners = self.change_listeners.read().await;
        for listener in listeners.iter() {
            listener(&value);
        }

        Ok(())
    }
}
```

Drivers can now use native async operations inside callbacks:

```rust
let wavelength = Parameter::new("wavelength_nm", 800.0)
    .connect_to_hardware_write({
        let port = self.port.clone();
        move |wavelength: f64| -> BoxFuture<'static, Result<()>> {
            Box::pin(async move {
                let mut p = port.lock().await;  // Async lock
                p.write_all(format!("WAVELENGTH:{}\r\n", wavelength).as_bytes()).await?;  // Async write
                tokio::time::sleep(Duration::from_millis(100)).await;  // Async delay
                Ok(())
            })
        }
    });
```

**Consequences:**

**Positive:**
- Native async support - no spawn_blocking or detached tasks
- Proper error propagation - `Parameter::set()` returns hardware errors
- Composable - callbacks can call other async functions naturally
- Testable - async operations can be mocked/tested properly
- Consistent - follows tokio ecosystem patterns

**Negative:**
- Requires `futures` crate for BoxFuture (small dependency)
- Slightly more verbose callback syntax (but much safer)
- Lifetime constraints require `'static` on BoxFuture (but this is correct - callbacks outlive the call)

**Alternatives Considered:**

1. **Keep sync callbacks + spawn_blocking**: Rejected - breaks error propagation and blocks threadpool
2. **Event queue approach**: Rejected - added complexity, delayed execution, harder to reason about
3. **Actor-based**: Rejected - V4 Kameo actors being removed, don't want to reintroduce actors

**Related Issues:**

- bd-s5ou: Implement async Parameter callbacks (BoxFuture)
- bd-hlr7: Wire hardware drivers to async parameters
- bd-dili: Migrate all hardware drivers to unified Parameter system
- bd-gcjl: Epic: V5 Reactive Parameter System Integration

**References:**

- Gemini architectural validation (2025-12-02)
- tokio async best practices: https://tokio.rs/tokio/tutorial
- BoxFuture pattern: https://docs.rs/futures/latest/futures/future/type.BoxFuture.html
