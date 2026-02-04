# driver-andor-sdk3

Safe Rust driver for Andor iStar camera and Shamrock spectrograph using Andor SDK3.

## Features

### Camera (Andor iStar)

- **Frame Acquisition**: Continuous streaming with circular buffers
- **External Triggering**: Support for external trigger inputs
- **MCP Gain Control**: Micro-Channel Plate intensifier gain (0-4095)
- **DDG Timing**: Digital Delay Generator for gate timing control
- **AOI & Binning**: Region of interest and binning configuration
- **Temperature Control**: Sensor cooling and monitoring

**Capabilities**: `FrameProducer`, `Triggerable`, `ExposureControl`, `Parameterized`

### Spectrograph (Andor Shamrock)

- **Wavelength Control**: Set center wavelength for grating
- **Grating Selection**: Switch between up to 3 gratings
- **Slit Width Control**: Adjust input/output slit widths
- **Flipper Mirror**: Direct vs side output selection
- **Wavelength Calibration**: Pixel-to-wavelength mapping

**Capabilities**: `WavelengthTunable`, `ShutterControl`, `Parameterized`

## Architecture

```
driver-andor-sdk3/
├── src/
│   ├── lib.rs          # Public API
│   ├── camera.rs       # iStar camera driver
│   ├── spectrograph.rs # Shamrock spectrograph driver
│   ├── mock.rs         # Mock implementations
│   ├── factory.rs      # DriverFactory impls
│   ├── types.rs        # Common types and enums
│   └── error.rs        # Error types
└── Cargo.toml
```

## Feature Flags

- `camera`: Enable iStar camera driver (default: off)
- `spectrograph`: Enable Shamrock spectrograph driver (default: off)
- `hardware`: Enable real SDK3 hardware (requires SDK installation, Windows only)
- Default: Mock implementations only (works on all platforms)

## Usage

### Camera Example

```rust
use driver_andor_sdk3::camera::AndorCamera;
use common::capabilities::{FrameProducer, Triggerable, ExposureControl};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create camera (camera_index = 0)
    let camera = AndorCamera::new_async(0).await?;

    // Configure for external triggering
    camera.set_trigger_mode("External").await?;
    camera.set_exposure(0.0015).await?;  // 1.5ms exposure
    camera.set_gate_mode("DDG").await?;
    camera.set_mcp_gain(3600).await?;
    camera.set_ddg_output_delay(1300000).await?;  // picoseconds
    camera.set_ddg_output_width(10000000).await?; // picoseconds

    // Start streaming
    camera.start_stream().await?;

    // ... process frames ...

    camera.stop_stream().await?;
    Ok(())
}
```

### Spectrograph Example

```rust
use driver_andor_sdk3::spectrograph::AndorSpectrograph;
use common::capabilities::{WavelengthTunable, ShutterControl};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create spectrograph (device_index = 0)
    let spec = AndorSpectrograph::new_async(0).await?;

    // Set grating and wavelength
    spec.set_grating(2).await?;
    spec.set_wavelength(310.0).await?;

    // Configure slits
    spec.set_slit_width(2, 150.0).await?;  // Port 2, 150µm

    // Get wavelength calibration for camera
    let calibration = spec.get_wavelength_calibration(2048).await?;

    println!("Wavelength range: {:?}", calibration.range());

    Ok(())
}
```

### TOML Configuration

```toml
[[devices]]
id = "istar_camera"
type = "andor_istar"
enabled = true

[devices.config]
camera_index = 0

[[devices]]
id = "shamrock_spec"
type = "andor_shamrock"
enabled = true

[devices.config]
device_index = 0
```

## Building

### Cross-Platform (Mock Mode)

```bash
cargo build -p driver-andor-sdk3
```

This builds with mock implementations that work on any platform (Linux, macOS, Windows).

### Windows with Real Hardware

```bash
# Set environment variable to SDK path
$env:ANDOR_SDK3_DIR = "C:\Program Files\Andor SDK3"

# Build with hardware features
cargo build -p driver-andor-sdk3 --features hardware
```

**Requirements**:
- Andor SDK3 installed (atcore.dll, atspectrograph.dll)
- ANDOR_SDK3_DIR environment variable set
- Windows only

## Testing

```bash
# Mock mode (all platforms)
cargo test -p driver-andor-sdk3

# Hardware mode (Windows only, with SDK installed)
cargo test -p driver-andor-sdk3 --features hardware
```

## Dependencies

- `andor-sdk3-sys`: FFI bindings to Andor SDK3
- `common`: DAQ framework capabilities and traits
- `tokio`: Async runtime
- `anyhow`, `thiserror`: Error handling
- `async-trait`: Async trait support

## Reference

Based on the Python initialization sequence in `LIBS/initialization.py` (lines 64-173), which demonstrates:
- Camera initialization and cooling
- AOI and binning configuration
- Trigger mode and exposure setup
- MCP gain and DDG timing control
- Spectrograph grating and wavelength setup
- Wavelength calibration retrieval

## License

MIT OR Apache-2.0
