# PyO3 Scripting Engine

## Overview

The PyO3 scripting engine provides Python scripting capabilities within rust-daq. It implements the `ScriptEngine` trait, allowing hot-swapping between different scripting backends (Rhai, Python, Lua, etc.).

## Features

- ✅ Execute Python scripts with full Python 3.x syntax
- ✅ Set and get global variables from Rust
- ✅ Type-safe value conversion between Rust and Python
- ✅ Async-compatible execution model
- ✅ Script validation without execution
- ✅ Thread-safe via `Arc<Mutex<>>`
- ✅ Automatic Python interpreter initialization

## Installation

Enable the `scripting_python` feature in your `Cargo.toml`:

```toml
[dependencies]
rust_daq = { version = "0.1", features = ["scripting_python"] }
```

Or build with:

```bash
cargo build --features scripting_python
```

## Basic Usage

```rust
use rust_daq::scripting::{ScriptEngine, PyO3Engine, ScriptValue};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new Python engine
    let mut engine = PyO3Engine::new()?;
    
    // Execute a simple script
    let script = r#"
x = 10
y = 20
result = x + y
print(f"Result: {result}")
"#;
    
    engine.execute_script(script).await?;
    
    // Get the result
    let result = engine.get_global("result")?;
    let value: i64 = result.downcast()?;
    println!("From Rust: {}", value); // Prints: 42
    
    Ok(())
}
```

## Setting Global Variables

```rust
use rust_daq::scripting::{ScriptEngine, PyO3Engine, ScriptValue};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = PyO3Engine::new()?;
    
    // Set variables from Rust
    engine.set_global("experiment_name", ScriptValue::new("Demo".to_string()))?;
    engine.set_global("num_samples", ScriptValue::new(1000_i64))?;
    engine.set_global("sampling_rate", ScriptValue::new(10.0_f64))?;
    
    // Use them in Python
    let script = r#"
print(f"Running experiment: {experiment_name}")
print(f"Samples: {num_samples}")
print(f"Rate: {sampling_rate} Hz")

duration = num_samples / sampling_rate
print(f"Duration: {duration} seconds")
"#;
    
    engine.execute_script(script).await?;
    
    // Get computed values back
    let duration = engine.get_global("duration")?;
    let dur_val: f64 = duration.downcast()?;
    println!("Experiment duration: {} seconds", dur_val);
    
    Ok(())
}
```

## Script Validation

Validate Python syntax without executing:

```rust
use rust_daq::scripting::{ScriptEngine, PyO3Engine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = PyO3Engine::new()?;
    
    // Valid script
    let valid = "x = 1 + 2";
    assert!(engine.validate_script(valid).await.is_ok());
    
    // Invalid script
    let invalid = "x = 1 +";  // Syntax error
    assert!(engine.validate_script(invalid).await.is_err());
    
    Ok(())
}
```

## Data Acquisition Example

```rust
use rust_daq::scripting::{ScriptEngine, PyO3Engine, ScriptValue};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = PyO3Engine::new()?;
    
    // Configure experiment parameters
    engine.set_global("laser_power", ScriptValue::new(5.0_f64))?;
    engine.set_global("exposure_time", ScriptValue::new(100_i64))?;
    
    // Run acquisition script
    let script = r#"
import time

# Acquisition loop
data = []
for i in range(10):
    # Simulate measurement
    measurement = laser_power * (1.0 + 0.1 * i)
    data.append(measurement)
    print(f"Sample {i}: {measurement:.2f} mW")

# Compute statistics
avg_power = sum(data) / len(data)
max_power = max(data)
min_power = min(data)

print(f"Average: {avg_power:.2f} mW")
print(f"Range: {min_power:.2f} - {max_power:.2f} mW")
"#;
    
    engine.execute_script(script).await?;
    
    // Get results
    let avg = engine.get_global("avg_power")?;
    let avg_val: f64 = avg.downcast()?;
    println!("Average power from Rust: {:.2} mW", avg_val);
    
    Ok(())
}
```

## Error Handling

The PyO3 engine provides detailed error information:

```rust
use rust_daq::scripting::{ScriptEngine, PyO3Engine, ScriptError};

#[tokio::main]
async fn main() {
    let mut engine = PyO3Engine::new().unwrap();
    
    let script = r#"
x = 10
y = 0
result = x / y  # Division by zero!
"#;
    
    match engine.execute_script(script).await {
        Ok(_) => println!("Success"),
        Err(ScriptError::RuntimeError { message, backtrace }) => {
            println!("Runtime error: {}", message);
            if let Some(bt) = backtrace {
                println!("Traceback:\n{}", bt);
            }
        }
        Err(e) => println!("Other error: {}", e),
    }
}
```

## Supported Types

The PyO3 engine supports automatic conversion for these types:

| Rust Type | Python Type |
|-----------|-------------|
| `String`  | `str`       |
| `i64`     | `int`       |
| `f64`     | `float`     |
| `bool`    | `bool`      |

## Thread Safety

The PyO3 engine is thread-safe and can be shared across async tasks:

```rust
use std::sync::Arc;
use tokio::sync::Mutex;
use rust_daq::scripting::{ScriptEngine, PyO3Engine, ScriptValue};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Arc::new(Mutex::new(PyO3Engine::new()?));
    
    let mut handles = vec![];
    
    for i in 0..5 {
        let engine_clone = engine.clone();
        let handle = tokio::spawn(async move {
            let mut eng = engine_clone.lock().await;
            eng.set_global("task_id", ScriptValue::new(i as i64)).unwrap();
            eng.execute_script("print(f'Task {task_id} running')").await.unwrap();
        });
        handles.push(handle);
    }
    
    for handle in handles {
        handle.await?;
    }
    
    Ok(())
}
```

## Comparison with Rhai Engine

| Feature | PyO3 (Python) | Rhai |
|---------|---------------|------|
| Syntax | Python 3.x | Rust-like |
| Performance | Moderate | Fast |
| Libraries | Full Python ecosystem | Limited |
| Learning Curve | Low (if you know Python) | Moderate |
| Binary Size | Larger | Smaller |
| Use Case | Complex analysis, prototyping | Embedded scripting, performance |

## Best Practices

1. **Use for complex analysis**: Python excels at data analysis with NumPy, SciPy, etc.
2. **Validate scripts early**: Use `validate_script()` before saving user scripts
3. **Clear globals between runs**: Call `clear_globals()` to avoid state leakage
4. **Handle errors gracefully**: Python errors include full tracebacks
5. **Consider performance**: For tight loops, use Rhai or native Rust

## Limitations

- Function registration requires Python-compatible functions (Py<PyAny>)
- Type conversion limited to basic types (String, i64, f64, bool)
- Python GIL may impact multi-threaded performance
- Requires Python 3.x to be installed on the system

## Future Enhancements

- [ ] Support for NumPy arrays
- [ ] Custom type registration
- [ ] Async Python function support
- [ ] Better error location reporting
- [ ] Python module import restrictions for security

## See Also

- [ScriptEngine Trait Documentation](script_engine.rs)
- [Rhai Engine Documentation](rhai_engine.rs)
- [PyO3 Official Documentation](https://pyo3.rs/)
