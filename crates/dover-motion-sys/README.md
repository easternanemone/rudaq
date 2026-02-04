# dover-motion-sys

Low-level FFI bindings for Dover Motion's MotionSynergyAPI C++ library.

## Overview

This crate provides Rust FFI bindings to the Dover Motion MotionSynergyAPI, which controls:

- **SmartStage™ XY** - Precision XY stages
- **SmartStage™ Linear** - Linear stages
- **DOF-5** - 5 degree-of-freedom stages
- **Dover Motion Control Module (DMCM)** - Motion control hardware

## Platform Support

| Platform | Status | Library | Notes |
|----------|--------|---------|-------|
| **Windows** | Primary | `MotionSynergyCore.dll` | Requires Visual C++ Redistributable |
| **Linux** | Secondary | `libMotionSynergyCore.so` | Tested on Ubuntu 22.04 LTS |
| macOS | Not supported | N/A | Dover Motion SDK is Windows/Linux only |

## Feature Flags

- **`dover-sdk`**: Enable real SDK bindings (requires Dover Motion SDK installed)
- **Default (no features)**: Use dummy bindings for development/CI without hardware

## Installation

### Windows Setup

1. Install Dover Motion MotionSynergyAPI SDK:
   - Default path: `C:\Program Files\Dover Motion\MotionSynergyAPI`
   - Includes headers (`.h`) and libraries (`.lib` / `.dll`)

2. Install [Microsoft Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)

3. Set environment variables (optional, if not using default paths):
   ```powershell
   $env:DOVER_SDK_DIR = "C:\Program Files\Dover Motion\MotionSynergyAPI"
   $env:DOVER_INCLUDE_DIR = "$env:DOVER_SDK_DIR\include"
   $env:DOVER_LIB_DIR = "$env:DOVER_SDK_DIR\lib"
   ```

4. Build with SDK feature:
   ```bash
   cargo build --features dover-sdk
   ```

5. **IMPORTANT**: Copy `MotionSynergyCore.dll` to your executable directory or add to `PATH`:
   ```powershell
   copy "$env:DOVER_LIB_DIR\MotionSynergyCore.dll" .\target\debug\
   ```

### Linux Setup

1. Install Dover Motion SDK (typically from vendor-provided package):
   ```bash
   # Example installation (actual steps may vary)
   sudo dpkg -i dover-motion-sdk_*.deb
   # Or manual installation
   sudo cp libMotionSynergyCore.so /usr/local/lib/
   sudo cp -r include/* /usr/local/include/dover-motion/
   sudo ldconfig
   ```

2. Install build dependencies:
   ```bash
   sudo apt-get install clang libclang-dev
   ```

3. Set environment variables (optional, if not using default paths):
   ```bash
   export DOVER_SDK_DIR=/usr/local
   export DOVER_INCLUDE_DIR=/usr/local/include/dover-motion
   export DOVER_LIB_DIR=/usr/local/lib
   ```

4. Build with SDK feature:
   ```bash
   cargo build --features dover-sdk
   ```

### Development/CI (No Hardware)

For development or CI environments without Dover Motion SDK:

```bash
cargo build  # No features - uses dummy bindings
cargo test   # Tests compile but don't call real SDK functions
```

## Usage

This is a low-level FFI crate. Most users should use the higher-level `driver-dover` crate.

### With Hardware

```toml
[dependencies]
dover-motion-sys = { version = "0.1", features = ["dover-sdk"] }
```

### Without Hardware (Mock Development)

```toml
[dependencies]
dover-motion-sys = "0.1"
```

## Critical Functions for LIBS Experiments

Based on Dover Motion API User Manual Section 6.2.2 (IAxisDevice):

### Trigger on Position (TOP)

```cpp
// Enable trigger pulses every 'increment' of travel
void EnableTriggerOnPosition(
    double startPosition,    // Position to start pulses (mm/um/nm)
    double endPosition,      // Position to stop pulses (mm/um/nm)
    double increment,        // Pulse every 'increment' of travel (mm/um/nm)
    bool isBidirectional,    // Pulse on return motion too?
    int pulseWidthNs         // Pulse width (50-204,800 ns, in 50ns steps)
);

void DisableTriggerOnPosition();
```

**Use case**: Trigger LIBS laser at precise positions during stage scan.

### Motion Control

```cpp
void MoveAbsolute(double position, double velocity);
void MoveRelative(double distance, double velocity);
void Stop();
void Home();
```

### Position Queries

```cpp
double GetActualPosition(bool forceRefresh = true);   // Encoder position
double GetCommandedPosition(bool forceRefresh = true); // Target position
```

### Velocity/Acceleration Control

```cpp
void SetVelocity(double velocity);
void SetAcceleration(double accel);
void SetDeceleration(double decel);
double GetVelocity(bool forceRefresh = true);
```

## Safety

All functions are `unsafe` as they directly call C++ FFI. Caller must ensure:

- Proper initialization: `Configure()` → `Connect()` → `Initialize()`
- Proper cleanup: `Shutdown()`
- Valid pointer lifetimes
- Thread safety (SDK may not be thread-safe)

## Windows Wide String Handling

Some Windows API functions use `WCHAR` (UTF-16). Consider using the `widestring` crate:

```rust
use widestring::U16CString;

let wide_str = U16CString::from_str("COM1").unwrap();
// Pass wide_str.as_ptr() to Windows API functions
```

## Testing

### Without Hardware

```bash
cargo test  # Tests dummy bindings
```

### With Hardware (Windows)

```powershell
# Ensure MotionSynergyCore.dll is in PATH or executable directory
cargo test --features dover-sdk -- --test-threads=1
```

### With Hardware (Linux)

```bash
export LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH
cargo test --features dover-sdk -- --test-threads=1
```

## CI/CD Integration

### GitHub Actions Example

```yaml
jobs:
  test-without-hardware:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build -p dover-motion-sys
      - run: cargo test -p dover-motion-sys

  test-with-hardware-windows:
    runs-on: [self-hosted, windows, dover-motion]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build -p dover-motion-sys --features dover-sdk
      - run: cargo test -p dover-motion-sys --features dover-sdk -- --test-threads=1

  test-with-hardware-linux:
    runs-on: [self-hosted, linux, dover-motion]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build -p dover-motion-sys --features dover-sdk
      - run: cargo test -p dover-motion-sys --features dover-sdk -- --test-threads=1
```

## Documentation References

- **Dover Motion - Motion Synergy API User Manual** (Document 102925)
  - Section 6: C++ Software Integration (pp. 121-163)
  - Section 6.2: Public Interface (pp. 127-163)
  - Section 7: Linux Support (pp. 164-166)

- **Key Classes**:
  - `imp::IAxisDevice` (Section 6.2.2) - Main axis control interface
  - `imp::MotionSynergyAPI` (Section 6.2.10) - Top-level API wrapper
  - `imp::CommunicationSettings` (Section 6.2.1) - Serial/CAN configuration

## License

MIT OR Apache-2.0

## Support

For Dover Motion SDK issues, contact Dover Motion support:
[Dover Software Support Request Form](https://dover.quickbase.com/db/brceq4k4h?a=nwr&originalQid=td)
