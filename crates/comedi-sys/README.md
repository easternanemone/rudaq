# comedi-sys

Low-level FFI bindings for the Linux Comedi library.

## Overview

The `comedi-sys` crate provides raw, unsafe Rust bindings to the comedilib C library, which is the user-space interface to Comedi (Control and Measurement Device Interface) - a collection of Linux kernel drivers for data acquisition hardware.

**Note**: For safe, idiomatic Rust code, use the `driver-comedi` crate instead.

## Comedi Hardware Support

Comedi supports a wide variety of data acquisition devices:

- **National Instruments** (NI) - PCI-6xxx, PCI-MIO-16, PCI-1200, etc.
- **Measurement Computing** - PCI-DAS, USB-DAS, etc.
- **Advantech** - PCL-818, PCL-726, etc.
- **ADDI-DATA** - APCIexx, APCxx series
- Many others via the Linux kernel comedi driver framework

For hardware inventory and setup, see `CLAUDE.md` or `docs/guides/comedi-setup.md`.

## Key Features

- **Raw FFI Bindings** - Direct access to comedilib C API
- **Constants and Types** - Enumerated subdevice types, I/O modes, etc.
- **Helper Macros** - `CR_PACK`, `CR_RANGE`, `CR_AREF` for channel/range/reference packing
- **Cross-Platform** - Uses pre-defined bindings for cross-compilation

## Feature Flags

- **`comedi-sdk`** (default: disabled) - Generate bindings from system comedilib headers via bindgen
  - Use for native compilation on a machine with comedilib installed
  - Requires `libcomedi-dev` package on Linux

- Without `comedi-sdk` - Use pre-defined bindings for cross-compilation
  - Faster compilation
  - Works on machines without comedilib installed

## Example (Unsafe)

```rust
use comedi_sys::*;
use std::ffi::CString;

unsafe {
    // Open device
    let path = CString::new("/dev/comedi0").unwrap();
    let dev = comedi_open(path.as_ptr());

    if !dev.is_null() {
        // Get number of subdevices
        let n_subdevices = comedi_get_n_subdevices(dev);
        println!("Device has {} subdevices", n_subdevices);

        // Read an analog input channel
        let data = 0u32;
        let result = comedi_data_read(dev, 0, 0, AREF_GROUND, &data);
        if result >= 0 {
            println!("Read value: {}", data);
        }

        // Close device
        comedi_close(dev);
    }
}
```

## Key Types and Constants

### Subdevice Types
```rust
COMEDI_SUBD_AI         // Analog input
COMEDI_SUBD_AO         // Analog output
COMEDI_SUBD_DI         // Digital input
COMEDI_SUBD_DO         // Digital output
COMEDI_SUBD_DIO        // Digital input/output (configurable)
COMEDI_SUBD_COUNTER    // Counter/timer
COMEDI_SUBD_TIMER      // Timer
COMEDI_SUBD_MEMORY     // Memory (calibration, etc.)
COMEDI_SUBD_CALIB      // Calibration subdevice
```

### I/O Directions
```rust
COMEDI_INPUT   // Read from device
COMEDI_OUTPUT  // Write to device
```

### Analog Reference Modes
```rust
AREF_GROUND    // Referenced to ground
AREF_COMMON    // Referenced to common
AREF_DIFF      // Differential (two channels)
AREF_OTHER     // Other (device-specific)
```

## Helper Functions

The following macros/functions help pack and unpack channel configuration:

```rust
// Pack channel, range, and analog reference into a single value
let packed = CR_PACK(channel, range, aref);

// Unpack individual fields
let chan = CR_CHAN(packed);
let rng = CR_RANGE(packed);
let aref = CR_AREF(packed);

// Pack with flags (for streaming operations)
let packed = CR_PACK_FLAGS(channel, range, aref, flags);
```

## Installation

Add to `Cargo.toml`:

```toml
[dependencies]
comedi-sys = { path = "../comedi-sys" }
```

Or with SDK feature for native compilation:

```toml
[dependencies]
comedi-sys = { path = "../comedi-sys", features = ["comedi-sdk"] }
```

## System Setup

### On Linux (Ubuntu/Debian)

```bash
# Install comedilib headers (for SDK feature)
sudo apt-get install libcomedi-dev

# Load kernel driver (if not already loaded)
modprobe comedi

# Check device
ls -la /dev/comedi*
```

### On macOS

Comedi is Linux-only. For cross-compilation, use the default (pre-defined) bindings without the `comedi-sdk` feature.

## Safety

All functions in this crate are `unsafe` because they are raw FFI bindings. When using `comedi-sys`, you must:

1. Verify device validity before use
2. Check return codes for errors
3. Ensure resources are properly freed
4. Handle concurrent access correctly

For safer abstractions, see the `driver-comedi` crate.

## External Resources

- [Comedi Project Home](http://www.comedi.org/)
- [comedilib Manual](http://www.comedi.org/doc/comedilib_8h.html)
- [Linux Comedi Driver Tree](https://github.com/comedi/comedi) - Kernel driver source
- [Comedi Examples](http://www.comedi.org/doc/) - C examples

## Related Documentation

- [Comedi Setup Guide](../../docs/guides/comedi-setup.md) - Full hardware setup
- [driver-comedi](../driver-comedi) - Safe Rust wrapper
- [Testing Guide](../../docs/guides/testing.md)

## See Also

- `driver-comedi` crate - Safe, idiomatic wrapper around comedi-sys
- `common` crate - Device traits and abstractions
