# common

Core abstraction layer for the rust-daq data acquisition system.

## Overview

`common` provides the foundational types, traits, and error handling for the entire rust-daq ecosystem. It enables:

- **Plugin Architecture** - Dynamic hardware driver registration via `DriverFactory`
- **Capability Composition** - Fine-grained traits for device capabilities
- **Reactive Parameters** - Observable state with validation and GUI metadata
- **Zero-Copy Data Flow** - Efficient frame handling with `bytes::Bytes`
- **Unified Error Model** - Categorized errors with recovery strategies

## Key Types

### Error Handling

```rust
use common::error::{DaqError, AppResult, DriverError, DriverErrorKind};

// Unified error type for all system errors
fn acquire_frame() -> AppResult<Frame> {
    // DaqError covers: Config, Instrument, Timeout, Processing, etc.
}

// Structured driver errors with categories
let err = DriverError::new(DriverErrorKind::Communication, "Device disconnected");
```

### Observable Parameters

```rust
use common::observable::Observable;

// Reactive parameter with validation
let wavelength = Observable::new(800.0)
    .with_name("wavelength")
    .with_units("nm")
    .with_range(700.0..=1000.0);

// Subscribe to changes
let mut rx = wavelength.subscribe();
wavelength.set(850.0)?;  // Validates against range
```

### Device Capabilities

Fine-grained traits for composable device behavior (23 total):

| Trait | Purpose |
|-------|---------|
| `Movable` | Position control (stages, rotators) |
| `Readable` | Scalar measurements (power meters, sensors) |
| `FrameProducer` | Image acquisition (cameras) |
| `FrameObserver` | Frame consumption and processing |
| `Triggerable` | External trigger support |
| `ShutterControl` | Shutter open/close |
| `WavelengthTunable` | Wavelength control (lasers, monochromators) |
| `EmissionControl` | Laser emission on/off |
| `ExposureControl` | Exposure time configuration |
| `Parameterized` | Device-specific settings access |
| `Settable` | Set scalar values (DAC output) |
| `Switchable` | Binary on/off control |
| `Actionable` | Trigger actions (reset, calibrate) |
| `Loggable` | Device logging and diagnostics |
| `Camera` | Camera-specific operations |
| `GatedCamera` | Gated/triggered cameras |
| `Stageable` | Multi-axis stage control |
| `Commandable` | Raw command interface |
| `SpectrometerControl` | Spectrometer configuration |
| `TriggerOnPosition` | Position-based triggering |
| `PulseGenerator` | Pulse/waveform generation |
| `SafetyInterlock` | Safety system integration |
| `Reconfigurable` | Runtime reconfiguration |

### Driver Factory (Plugin System)

```rust
use common::driver::{DriverFactory, DeviceComponents, Capability};

pub struct MyDriverFactory;

impl DriverFactory for MyDriverFactory {
    fn driver_type(&self) -> &'static str { "my_device" }
    fn name(&self) -> &'static str { "My Custom Device" }
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Readable, Capability::Movable]
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(MyDriver::new(&config).await?);
            Ok(DeviceComponents::new()
                .with_readable(driver.clone())
                .with_movable(driver))
        })
    }
}
```

### Frame Data

```rust
use common::data::{Frame, FrameMetadata, PixelBuffer};

// Zero-copy frame with metadata
let frame = Frame {
    pixels: PixelBuffer::U16(data),
    width: 2048,
    height: 2048,
    metadata: FrameMetadata {
        frame_number: 1,
        timestamp_ns: now,
        exposure_ms: 100.0,
        ..Default::default()
    },
};
```

## Resource Limits

Built-in DoS prevention:

```rust
use common::limits::{validate_frame_size, MAX_FRAME_BYTES, MAX_SCRIPT_SIZE};

// Validates dimensions don't overflow (returns safe byte count)
let size = validate_frame_size(width, height, bytes_per_pixel)?;
```

| Limit | Value | Purpose |
|-------|-------|---------|
| `MAX_FRAME_BYTES` | 100 MB | Maximum frame size |
| `MAX_SCRIPT_SIZE` | 1 MB | Maximum Rhai script |
| `MAX_FRAME_DIMENSION` | 65,536 | Maximum width/height |
| `RPC_TIMEOUT` | 15s | gRPC call timeout |

## Error Recovery

Errors are categorized for appropriate handling:

| Error Type | Recovery Strategy |
|------------|-------------------|
| `Config`, `FeatureNotEnabled` | Fix configuration, rebuild |
| `Timeout`, `ModuleBusy` | Retry with exponential backoff |
| `Instrument`, `Serial*` | Check connections, retry |
| `Processing` | Skip frame, continue acquisition |

## Feature Flags

- `serial` - Enable serial port support (via `serial2-tokio`)
- `storage_arrow` - Enable Arrow IPC format support
- `storage_hdf5` - Enable shared HDF5 types/error support

## Module Organization

```
src/
├── error.rs         # DaqError, DriverError, AppResult
├── observable.rs    # Observable<T> with validation
├── parameter.rs     # Parameter<T> with hardware callbacks
├── capabilities.rs  # Movable, Readable, FrameProducer, etc.
├── driver.rs        # DriverFactory, DeviceComponents
├── data.rs          # Frame, PixelBuffer, Metadata
├── pipeline.rs      # MeasurementSource/Sink/Processor
├── limits.rs        # Resource limits and validation
└── health/          # SystemHealthMonitor
```

## Related Crates

- [`hardware`](../hardware) - Hardware abstraction layer using these traits
- [`server`](../server) - gRPC server exposing capabilities
- [`driver-*`](../driver-pvcam) - Driver implementations using `DriverFactory`

## License

See the repository root for license information.
