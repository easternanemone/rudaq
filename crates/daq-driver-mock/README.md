# daq-driver-mock

Mock hardware drivers for testing without physical hardware.

## Features

### Operational Modes

- **Instant**: Zero delays, deterministic behavior (unit tests)
- **Realistic**: Hardware-like timing (integration tests)
- **Chaos**: Configurable failures (resilience testing)

### Available Devices

| Device | Capabilities | Real Hardware |
|--------|-------------|---------------|
| MockCamera | FrameProducer, Triggerable | PVCAM cameras |
| MockStage | Movable | ESP300, linear stages |
| MockPowerMeter | Readable, WavelengthTunable | Newport 1830-C |
| MockLaser | WavelengthTunable, ShutterControl, EmissionControl | MaiTai Ti:Sapphire |
| MockRotator | Movable | ELL14 rotary stages |
| MockDAQOutput | Settable | Comedi analog outputs |

### Builder Pattern

All mock devices use a builder pattern for flexible configuration:

```rust
let camera = MockCamera::builder()
    .resolution(2048, 2048)
    .mode(MockMode::Realistic)
    .frame_loss_rate(0.01)
    .error_injection(ErrorConfig::random_failures(0.001))
    .build();

let laser = MockLaser::builder()
    .initial_wavelength(800.0)
    .mode(MockMode::Instant)
    .error_injection(ErrorConfig::none())
    .build();

let rotator = MockRotator::builder()
    .initial_position(45.0)
    .mode(MockMode::Realistic)
    .build();
```

### Error Injection

Configure realistic failure scenarios for resilience testing:

```rust
// Random failures
let config = ErrorConfig::random_failures(0.01);

// Specific scenario
let config = ErrorConfig::scenario(ErrorScenario::FailAfterN {
    operation: "move",
    count: 5,
});

// Reproducible with seed
let config = ErrorConfig::random_failures_seeded(0.1, Some(42));

// Multiple scenarios
let config = ErrorConfig::scenarios(vec![
    ErrorScenario::FailAfterN { operation: "read", count: 10 },
    ErrorScenario::Timeout { operation: "move" },
]);
```

### Mock Modes

#### Instant Mode
Zero-delay operations for fast unit tests:
```rust
let stage = MockStage::builder()
    .mode(MockMode::Instant)
    .build();

stage.move_abs(100.0).await?;  // Returns immediately
```

#### Realistic Mode
Hardware-like timing for integration tests:
```rust
let camera = MockCamera::builder()
    .resolution(1024, 1024)
    .mode(MockMode::Realistic)
    .build();

camera.start_stream().await?;  // 33ms frame readout delay
```

#### Chaos Mode
Random errors and timing variations:
```rust
let meter = MockPowerMeter::builder()
    .mode(MockMode::Chaos)
    .error_injection(ErrorConfig::random_failures(0.05))
    .build();

// ~5% of operations will fail randomly
let power = meter.read().await?;
```

## Device-Specific Features

### MockCamera
- Configurable resolution (width × height)
- Test pattern generation (gradient, checkerboard)
- Frame loss simulation
- Trigger support

### MockStage
- Linear motion with realistic timing (10 mm/s)
- Soft limits and bounds checking
- Velocity profiles
- Settling time simulation

### MockPowerMeter
- Wavelength-dependent power readings
- Configurable noise (~1%)
- Units support (W, mW, µW)
- Optical filters simulation

### MockLaser
- Wavelength tuning (690-1040 nm, MaiTai range)
- Warmup time simulation (30s in Realistic mode)
- Safety interlocks (shutter must be open before emission)
- Shutter and emission independent controls

### MockRotator
- Full 360° rotation
- Velocity control (0-100%)
- Velocity-dependent motion timing
- Position wrapping (0-360°)

### MockDAQOutput
- Multiple voltage ranges (±10V, ±5V, 0-10V, 0-5V)
- Range validation
- Channel configuration
- Settling time simulation

## Driver Factory Pattern

All mock devices implement `DriverFactory` for automatic registration:

```rust
use daq_driver_mock::{MockCameraFactory, MockLaserFactory};
use daq_hardware::DeviceRegistry;

let registry = DeviceRegistry::new();
registry.register_factory(Box::new(MockCameraFactory));
registry.register_factory(Box::new(MockLaserFactory));
// ... or use register_all()
daq_driver_mock::register_all(&registry);
```

## TOML Configuration

Mock devices can be configured via TOML hardware configs:

```toml
[[devices]]
id = "mock_camera"
type = "mock_camera"
enabled = true
[devices.config]
width = 2048
height = 2048
mode = "realistic"

[[devices]]
id = "mock_laser"
type = "mock_laser"
enabled = true
[devices.config]
initial_wavelength = 800.0
mode = "instant"

[[devices]]
id = "mock_rotator"
type = "mock_rotator"
enabled = true
[devices.config]
initial_position = 0.0
mode = "realistic"
```

See `config/demo_mock_all.toml` for a complete example.

## Testing

Run mock driver tests:
```bash
cargo nextest run -p daq-driver-mock
cargo nextest run --test integration -p daq-driver-mock
```

Use with daemon:
```bash
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo_mock_all.toml
```

## Performance Characteristics

| Device | Operation | Instant | Realistic |
|--------|-----------|---------|-----------|
| Camera | Frame readout | 0ms | 33ms (~30fps) |
| Stage | 10mm move | 0ms | 1000ms (10mm/s) |
| PowerMeter | Read value | 0ms | 10ms |
| Laser | Wavelength tune | 0ms | 100ms |
| Rotator | 90° rotation | 0ms | 500ms (velocity-dependent) |
| DAQOutput | Set voltage | 0ms | 5ms |

## See Also

- [DEMO.md](../../DEMO.md) - Quick start with mock devices
- [Testing Guide](../../docs/guides/testing.md) - Testing patterns
