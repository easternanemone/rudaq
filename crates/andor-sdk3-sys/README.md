# andor-sdk3-sys

Low-level FFI bindings for the Andor SDK3 (camera and spectrograph control).

## Overview

This crate provides unsafe Rust bindings to the Andor SDK3 C library, which supports:
- **Cameras**: Andor Neo, Zyla, Marana scientific cameras
- **Spectrographs**: Shamrock series imaging spectrographs

The SDK uses a property-based interface with UTF-16 wide strings (Windows).

## Features

- `andor-sdk3` - Generate bindings from SDK headers (requires SDK installation)
- `camera` - Enable camera API functions (atcore.dll)
- `spectrograph` - Enable spectrograph API functions (atspectrograph.dll)
- `hardware` - Enable all hardware features (camera + spectrograph + SDK)

## Platform Support

**Windows only** - The Andor SDK3 is Windows-only. This crate will compile on other platforms with dummy bindings for cross-compilation, but will panic at runtime if hardware functions are called.

## Environment Variables

When building with the `andor-sdk3` feature enabled:

- `ANDOR_SDK3_DIR` - Path to Andor SDK3 installation (e.g., `C:\Program Files\Andor SDK3`)

## Installation

1. Install Andor SDK3 from Andor Technology website
2. Set environment variable:
   ```powershell
   $env:ANDOR_SDK3_DIR = "C:\Program Files\Andor SDK3"
   ```
3. Build with hardware feature:
   ```bash
   cargo build --features hardware
   ```

## API Structure

### Camera API (atcore.dll)

Core functions for camera control:
- `AT_InitialiseLibrary()` / `AT_FinaliseLibrary()` - Library lifecycle
- `AT_Open()` / `AT_Close()` - Device connection
- `AT_SetInt()` / `AT_GetInt()` - Integer properties
- `AT_SetFloat()` / `AT_GetFloat()` - Float properties
- `AT_SetEnumString()` / `AT_GetEnumString()` - Enum properties (UTF-16)
- `AT_Command()` - Execute commands (e.g., "AcquisitionStart")
- `AT_QueueBuffer()` / `AT_WaitBuffer()` - Frame acquisition

### Spectrograph API (atspectrograph.dll)

Functions for spectrograph control:
- `ATSpectrograph_InitialiseLibrary()` / `ATSpectrograph_Close()` - Library lifecycle
- `ATSpectrograph_SetGrating()` / `ATSpectrograph_GetGrating()` - Grating selection
- `ATSpectrograph_SetWavelength()` / `ATSpectrograph_GetWavelength()` - Center wavelength
- `ATSpectrograph_SetSlitWidth()` - Entrance/exit slit width
- `ATSpectrograph_GetCalibration()` - Wavelength calibration array
- `ATSpectrograph_SetShutter()` - Shutter control

## Wide String Handling

The Andor SDK3 uses UTF-16 wide strings for feature names and enum values. Helper functions are provided:

```rust
use andor_sdk3_sys::*;

unsafe {
    // Convert Rust string to wide string
    let feature = to_wide_string("SensorWidth");
    let mut width: AT_64 = 0;
    AT_GetInt(handle, feature.as_ptr(), &mut width);

    // Receive wide string from SDK
    let mut buffer = wide_string_buffer(256);
    AT_GetEnumStringByIndex(handle, feature.as_ptr(), 0, buffer.as_mut_ptr(), 256);
    let value = from_wide_string(&buffer);
}
```

## Safety

All functions are `unsafe` as they are direct FFI bindings. Callers must ensure:
- Library is initialized before use
- Handles are valid
- Buffers are correctly sized
- Wide strings are null-terminated
- Memory is properly managed

For a safe wrapper, use the `driver-andor-sdk3` crate instead.

## Example

```rust
use andor_sdk3_sys::*;

unsafe {
    // Initialize library
    let ret = AT_InitialiseLibrary();
    assert_eq!(ret, AT_SUCCESS);

    // Open first camera
    let mut handle = AT_HANDLE_UNINITIALISED;
    let ret = AT_Open(0, &mut handle);
    assert_eq!(ret, AT_SUCCESS);

    // Get sensor dimensions
    let width_feature = to_wide_string("SensorWidth");
    let height_feature = to_wide_string("SensorHeight");
    let mut width: AT_64 = 0;
    let mut height: AT_64 = 0;
    AT_GetInt(handle, width_feature.as_ptr(), &mut width);
    AT_GetInt(handle, height_feature.as_ptr(), &mut height);
    println!("Sensor: {}x{}", width, height);

    // Start acquisition
    let cmd = to_wide_string("AcquisitionStart");
    AT_Command(handle, cmd.as_ptr());

    // Queue buffer for frame
    let frame_size = (width * height * 2) as usize; // 16-bit pixels
    let mut buffer = vec![0u8; frame_size];
    AT_QueueBuffer(handle, buffer.as_mut_ptr(), frame_size as i32);

    // Wait for frame (5 second timeout)
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut size: i32 = 0;
    AT_WaitBuffer(handle, &mut ptr, &mut size, 5000);

    // Stop acquisition
    let cmd = to_wide_string("AcquisitionStop");
    AT_Command(handle, cmd.as_ptr());

    // Cleanup
    AT_Close(handle);
    AT_FinaliseLibrary();
}
```

## References

- [Andor SDK3 Documentation](https://andor.oxinst.com/products/software-development-kit/andor-sdk3)
- [pyAndorSDK3](https://github.com/MolecularTheoryGroup/pyAndorSDK3) - Python wrapper reference

## License

MIT OR Apache-2.0
