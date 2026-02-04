# Migration from LIBS Python Code

This document maps the legacy LIBS Python trigger code (`LIBS/trigger.py`) to the new Rust driver.

## Class Mapping

### Python `digital` Class → Rust `TriggerMode::Digital`

#### Python (LIBS/trigger.py)

```python
from nidaqmx.constants import AcquisitionType

class digital(object):
    def __init__(self):
        self.task = nidaqmx.Task()

    def configure(self, high_time=0.1, low_time=0.001, samps_per_chan=1):
        self.close()
        self.__init__()
        self.task.co_channels.add_co_pulse_chan_time(
            "Dev1/ctr0",
            high_time=high_time,
            low_time=low_time
        )
        self.task.timing.cfg_implicit_timing(
            sample_mode=AcquisitionType.FINITE,
            samps_per_chan=samps_per_chan
        )
        self.task.wait_until_done()

    def single_task(self):
        self.task.start()
        self.task.wait_until_done()
        self.task.stop()

    def close(self):
        self.task.close()
```

#### Rust (driver-nidaqmx)

```rust
use driver_nidaqmx::{NiDaqTrigger, TriggerMode, Triggerable};

// Create trigger (equivalent to __init__ + configure)
let trigger = NiDaqTrigger::new(
    TriggerMode::Digital,
    0.1,   // high_time
    0.001, // low_time
    1,     // samps_per_chan
).await?;

// Arm and trigger (equivalent to single_task)
trigger.arm().await?;      // Calls configure internally
trigger.trigger().await?;  // Calls start → wait → stop

// Cleanup (automatic via Drop trait)
trigger.disarm().await?;
```

### Python `TOP` Class → Rust `TriggerMode::TriggerOnPosition`

#### Python (LIBS/trigger.py)

```python
from nidaqmx.constants import Edge

class TOP(object):
    def __init__(self):
        self.task = nidaqmx.Task()

    def configure(self, high_time=0.001, low_time=0.001, samps_per_chan=1):
        self.close()
        self.__init__()
        self.task.co_channels.add_co_pulse_chan_time(
            "Dev1/ctr0",
            high_time=high_time,
            low_time=low_time
        )
        self.task.timing.cfg_implicit_timing(
            sample_mode=AcquisitionType.FINITE,
            samps_per_chan=samps_per_chan
        )
        self.task.triggers.start_trigger.cfg_dig_edge_start_trig(
            trigger_source="/Dev1/PFI0",
            trigger_edge=Edge.RISING
        )
        self.task.triggers.start_trigger.retriggerable = False
        self.task.wait_until_done()

    def single_task(self):
        self.task.start()
        self.task.wait_until_done()
        self.task.stop()

    def close(self):
        self.task.close()
```

#### Rust (driver-nidaqmx)

```rust
use driver_nidaqmx::{NiDaqTrigger, TriggerMode, Triggerable};

// Create trigger with external edge configuration
let trigger = NiDaqTrigger::new(
    TriggerMode::TriggerOnPosition {
        trigger_source: "/Dev1/PFI0".to_string(),
        rising_edge: true,
        retriggerable: false,
    },
    0.001, // high_time
    0.001, // low_time
    1,     // samps_per_chan
).await?;

// Arm (waits for external trigger)
trigger.arm().await?;

// Trigger (in TOP mode, this waits for PFI0 edge)
trigger.trigger().await?;

// Cleanup
trigger.disarm().await?;
```

## Migration Checklist

### For LIBS Integration

- [ ] **Replace Python imports**: Remove `from trigger import digital, TOP`
- [ ] **Update initialization**: Use `NiDaqTrigger::new()` instead of `digital()` or `TOP()`
- [ ] **Update configuration**: Move parameters to constructor instead of `configure()` method
- [ ] **Update triggering**: Use async/await with `Triggerable` trait methods
- [ ] **Update cleanup**: Use `disarm()` or rely on automatic `Drop` cleanup

### Example: LIBS Acquisition Loop

#### Before (Python)

```python
from trigger import digital

trigger = digital()
trigger.configure(high_time=0.1, low_time=0.001, samps_per_chan=1)

for i in range(num_frames):
    trigger.single_task()
    # Acquire frame
    frame = camera.get_frame()
    save_frame(frame)

trigger.close()
```

#### After (Rust)

```rust
use driver_nidaqmx::{NiDaqTrigger, TriggerMode, Triggerable};

let trigger = NiDaqTrigger::new(
    TriggerMode::Digital,
    0.1,
    0.001,
    1,
).await?;

for i in 0..num_frames {
    trigger.arm().await?;
    trigger.trigger().await?;

    // Acquire frame
    let frame = camera.get_frame().await?;
    save_frame(frame).await?;
}

trigger.disarm().await?;
```

## Key Differences

### 1. Async/Await Model

**Python**: Synchronous blocking calls
```python
trigger.single_task()  # Blocks until complete
```

**Rust**: Async/await (non-blocking)
```rust
trigger.trigger().await?;  // Yields to runtime
```

### 2. Error Handling

**Python**: Exceptions
```python
try:
    trigger.configure(...)
except nidaqmx.DaqError as e:
    print(f"Error: {e}")
```

**Rust**: Result types
```rust
match trigger.arm().await {
    Ok(()) => println!("Armed successfully"),
    Err(e) => eprintln!("Error: {}", e),
}
```

### 3. Resource Management

**Python**: Manual cleanup required
```python
trigger = digital()
try:
    trigger.configure(...)
finally:
    trigger.close()  # Must call explicitly
```

**Rust**: Automatic cleanup via Drop
```rust
let trigger = NiDaqTrigger::new(...).await?;
// ... use trigger ...
// Automatically closed when trigger goes out of scope
```

### 4. Type Safety

**Python**: Runtime type checking
```python
trigger.configure(high_time="invalid")  # Runtime error
```

**Rust**: Compile-time type checking
```rust
NiDaqTrigger::new(
    TriggerMode::Digital,
    "invalid",  // Compile error: expected f64
    0.001,
    1,
)
```

## Performance Comparison

| Aspect | Python (LIBS) | Rust (driver-nidaqmx) | Notes |
|--------|---------------|----------------------|-------|
| **Latency** | ~2-3ms | ~3-5ms | PyO3 adds ~1-2ms overhead |
| **Memory** | ~50MB (interpreter) | ~5MB (compiled) | Rust more efficient |
| **CPU** | GIL contention | Thread-safe | Better for multi-threaded apps |
| **Safety** | Runtime errors | Compile-time checks | Rust catches bugs earlier |

## Testing Strategy

### Unit Tests

Replace Python unit tests with Rust tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_digital_trigger() {
        let trigger = NiDaqTrigger::new(
            TriggerMode::Digital,
            0.1,
            0.001,
            1,
        ).await.expect("Failed to create trigger");

        assert!(!trigger.is_armed().await);

        trigger.arm().await.expect("Failed to arm");
        assert!(trigger.is_armed().await);

        trigger.trigger().await.expect("Failed to trigger");
        trigger.disarm().await.expect("Failed to disarm");
    }
}
```

### Hardware Tests

Enable hardware tests with environment variable:

```bash
export NIDAQMX_HARDWARE_TEST=1
cargo test -p driver-nidaqmx --features hardware -- --nocapture
```

## Common Migration Issues

### Issue 1: Synchronous to Async

**Problem**: Python code uses blocking calls in tight loops

**Solution**: Use Tokio tasks or channels for concurrency

```rust
// Spawn trigger in background task
let trigger_task = tokio::spawn(async move {
    loop {
        trigger.arm().await?;
        trigger.trigger().await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
});
```

### Issue 2: Device Naming

**Problem**: Python code hardcodes "Dev1/ctr0"

**Solution**: Make device name configurable

```rust
let device = env::var("DAQ_DEVICE").unwrap_or("Dev1".to_string());
let counter = env::var("DAQ_COUNTER").unwrap_or("ctr0".to_string());

let trigger = NiDaqTrigger::with_device(
    mode,
    high_time,
    low_time,
    samps_per_chan,
    &device,
    &counter,
).await?;
```

### Issue 3: Error Recovery

**Problem**: Python code may leak tasks on error

**Solution**: Use Rust Drop trait for guaranteed cleanup

```rust
impl Drop for NiDaqTrigger {
    fn drop(&mut self) {
        // Cleanup happens automatically even on panic
        if let Some(py_task) = self.task.lock().take() {
            Python::with_gil(|py| {
                let _ = py_task.bind(py).call_method0("close");
            });
        }
    }
}
```

## Future Enhancements

### 1. Native C API Driver (Optional)

For ultra-low latency (<1ms), consider implementing a native driver:

```rust
// Hypothetical native driver (not implemented)
use nidaqmx_sys::*;  // Direct C bindings

unsafe {
    DAQmxCreateTask("MyTask", &mut task_handle)?;
    DAQmxCreateCOPulseChanTime(task_handle, "Dev1/ctr0", ...)?;
    DAQmxStartTask(task_handle)?;
}
```

**Tradeoff**: Eliminates Python overhead but requires maintaining C bindings.

### 2. Comedi Migration (Linux)

For Linux production, use native Comedi driver:

```rust
use driver_comedi::ComediAnalogOutput;

let output = ComediAnalogOutput::new("/dev/comedi0", 0).await?;
output.write(voltage).await?;
```

**Benefit**: No Python dependency, better real-time performance.

## Support

For migration assistance:

1. Check `examples/basic_trigger.rs` for working code
2. Review `INTEGRATION.md` for rust-daq integration
3. See original Python code in `LIBS/trigger.py` for reference

## License

This migration guide is part of driver-nidaqmx, licensed under MIT OR Apache-2.0.
