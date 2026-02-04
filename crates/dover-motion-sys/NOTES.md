# Implementation Notes for dover-motion-sys

## Design Decisions

### Why Dummy Bindings Instead of Conditional Compilation?

Following the `comedi-sys` pattern, this crate provides **dummy bindings** when the `dover-sdk` feature is disabled, rather than using conditional compilation (`#[cfg(...)]`).

**Advantages:**
1. **Workspace builds succeed on all platforms** - CI/CD can build the entire workspace without Dover Motion SDK
2. **Runtime panic on misuse** - If someone accidentally calls FFI functions without the SDK, they get a clear panic message at runtime
3. **Type checking always works** - IDEs and rust-analyzer can provide completions even without SDK
4. **Consistent API surface** - Driver code doesn't need `#[cfg]` everywhere

**Trade-off:**
- Slightly larger binary size (includes panic stubs)
- Runtime error instead of compile-time error for SDK misuse

This is acceptable because:
- The high-level driver (`driver-dover`) will handle feature flags correctly
- End users interact with the driver crate, not this FFI crate directly

### Platform Support Strategy

**Windows = Primary**: Dover Motion SDK is primarily designed for Windows.

**Linux = Secondary**: Linux support exists but is less common in the field.

**macOS = Not Supported**: Dover Motion SDK does not support macOS.

The build script detects the platform and configures paths accordingly:

```rust
match target_os.as_str() {
    "windows" => { /* Windows-specific config */ },
    "linux" => { /* Linux-specific config */ },
    _ => panic!("Unsupported OS"),
}
```

### C++ Binding Challenges

Dover Motion uses a **C++ API**, not C. This introduces complexity:

1. **Name Mangling**: C++ compilers mangle function names. Bindgen handles this with `-std=c++17`.

2. **Namespaces**: The `imp::` namespace is preserved using `.enable_cxx_namespaces()`.

3. **Classes vs Structs**: C++ classes become opaque Rust types. Method calls require C-style function wrappers or C++ interop layer.

4. **RAII**: C++ constructors/destructors don't map to Rust. The driver must manually call `Initialize()` / `Shutdown()`.

### Trigger on Position (TOP) - CRITICAL for LIBS

The `EnableTriggerOnPosition()` function is the **most important** function for LIBS experiments:

```cpp
void EnableTriggerOnPosition(
    double startPosition,    // Where to start triggering
    double endPosition,      // Where to stop triggering
    double increment,        // Trigger every X mm/um/nm
    bool isBidirectional,    // Trigger on return trip?
    int pulseWidthNs         // GPIO pulse width (50-204,800 ns)
);
```

**Use case**: As the stage scans across a sample, trigger the LIBS laser at precise intervals (e.g., every 100 µm).

**GPIO configuration**: Must set `GpioConfiguration` to `PMDGpioConfiguration::S2` in `Instrument.cfg`.

**Pulse width**: Hardware limitation - must be multiple of 50 ns, max 204,800 ns. Firmware v1.2+ only.

## Testing Strategy

### Without SDK (Default)

Tests verify that:
1. Dummy bindings compile
2. Types are defined correctly
3. Constants have correct values
4. No link errors (panic stubs are present)

### With SDK (`dover-sdk` Feature)

Tests verify that:
1. Real bindings generate successfully from headers
2. Types match expected layout
3. Functions are callable (but don't require hardware)

### With Hardware (Manual Testing)

Real hardware tests must be done manually or on self-hosted CI runners:
1. Initialize hardware
2. Move to absolute position
3. Verify encoder reads correct position
4. Enable TOP and verify GPIO pulses
5. Shutdown hardware

## Future Enhancements

### 1. C++ Wrapper Layer (Optional)

If bindgen has trouble with complex C++ classes, consider creating a C wrapper:

```cpp
// dover_c_api.h
extern "C" {
    IAxisDevice* dover_create_axis(const char* name);
    void dover_move_absolute(IAxisDevice* axis, double position, double velocity);
    void dover_destroy_axis(IAxisDevice* axis);
}
```

Then bind to the C API instead of C++.

### 2. Wide String Utilities

If Windows APIs require `WCHAR`, add helper functions:

```rust
use widestring::U16CString;

pub fn to_wide_string(s: &str) -> U16CString {
    U16CString::from_str(s).expect("Invalid UTF-16")
}
```

### 3. Error Code Mapping

Dover Motion SDK likely returns error codes. Map them to Rust enums:

```rust
#[repr(i32)]
pub enum DoverError {
    Success = 0,
    NotInitialized = -1,
    InvalidParameter = -2,
    // ...
}
```

### 4. Async Wrapper

Consider an async wrapper for long operations (homing, moves):

```rust
pub async fn move_absolute_async(axis: *mut IAxisDevice, pos: f64) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        unsafe { dover_move_absolute(axis, pos) }
    }).await??;
    Ok(())
}
```

## Known Limitations

1. **SDK Headers Not Included**: This crate does NOT include Dover Motion SDK headers. Users must install the SDK separately.

2. **No Cross-Compilation**: Building for Windows from Linux (or vice versa) is not supported because bindgen needs actual SDK headers.

3. **Single-Threaded**: The SDK may not be thread-safe. Use `--test-threads=1` for tests.

4. **DLL Deployment**: Windows users must manually copy `MotionSynergyCore.dll` to executable directory or add to PATH.

## References

- **Dover Motion API User Manual** (Document 102925)
  - Section 6.2.2: IAxisDevice Class (pp. 128-153)
  - Section 6.2.10: MotionSynergyAPI Class (pp. 154-156)
  - Section 7: Linux Support (pp. 164-166)

- **Key API Methods**:
  - EnableTriggerOnPosition (p. 130-131)
  - MoveAbsolute/MoveRelative (p. 141)
  - GetActualPosition (p. 130)
  - SetVelocity (p. 152)

- **GPIO Configuration**:
  - Section 6.2.2.1.5: Trigger on Position details
  - Requires PMDGpioConfiguration::S2 in Instrument.cfg

## Contact

For Dover Motion SDK issues:
- [Dover Software Support Request Form](https://dover.quickbase.com/db/brceq4k4h?a=nwr&originalQid=td)
