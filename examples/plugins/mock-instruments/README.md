# Mock Instruments Plugin

Simulated hardware devices for testing rust-daq without physical hardware.

## Overview

This plugin provides mock implementations of common laboratory instruments, enabling:

- **Development**: Test application logic without hardware
- **CI/CD Integration**: Run automated tests without physical devices
- **Demos & Training**: Show system capabilities without setup complexity
- **Integration Testing**: Verify data pipelines and processing logic

## Included Devices

### MockStage (Movable)

Simulated linear motion stage with realistic timing characteristics.

| Parameter | Value | Description |
|-----------|-------|-------------|
| Speed | 10 mm/s | Motion velocity |
| Settling time | 50 ms | Post-motion delay |
| Position range | ±∞ | No limits (simulated) |

**Capabilities**: `Movable`, `Parameterized`

### MockPowerMeter (Readable)

Simulated power meter with configurable base reading and realistic jitter.

| Parameter | Value | Description |
|-----------|-------|-------------|
| Jitter | ~1% | Random noise on readings |
| Range | 0-10 W | Configurable base value |
| Unit | Watts | Power measurement |

**Capabilities**: `Readable`, `Parameterized`

## Quick Start

### 1. Add to Hardware Configuration

Copy device definitions to your `hardware.toml`:

```toml
# Mock stage for testing
[[devices]]
id = "mock_stage"
name = "Mock Stage"
[devices.driver]
type = "mock_stage"
initial_position = 0.0

# Mock sensor for testing
[[devices]]
id = "mock_sensor"
name = "Mock Power Meter"
[devices.driver]
type = "mock_power_meter"
reading = 1.0e-6
```

### 2. Start the Daemon

```bash
rust-daq daemon --hardware-config hardware.toml
```

### 3. Use in Scripts

**Rhai Script:**
```rhai
let stage = devices.get("mock_stage");
stage.move_abs(10.0);
stage.wait_settled();
let pos = stage.position();
print(`Position: ${pos} mm`);

let meter = devices.get("mock_sensor");
let reading = meter.read();
print(`Power: ${reading} W`);
```

**Python Client:**
```python
stage = client.get_device("mock_stage")
await stage.move_abs(10.0)
await stage.wait_settled()
pos = await stage.position()
print(f"Position: {pos} mm")

meter = client.get_device("mock_sensor")
reading = await meter.read()
print(f"Power: {reading:.6e} W")
```

## CI/CD Integration

Mock devices enable automated testing without hardware dependencies.

### GitHub Actions Example

```yaml
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Start daemon with mock hardware
        run: |
          rust-daq daemon \
            --hardware-config examples/plugins/mock-instruments/mock_stage.toml &
          sleep 2

      - name: Run integration tests
        run: cargo test --features integration
```

### Test Assertions

```rust
#[tokio::test]
async fn test_mock_stage_motion() {
    let stage = get_device::<MockStage>("mock_stage").await;

    stage.move_abs(10.0).await.unwrap();
    stage.wait_settled().await.unwrap();

    let pos = stage.position().await.unwrap();
    assert!((pos - 10.0).abs() < 0.001);
}

#[tokio::test]
async fn test_mock_sensor_jitter() {
    let meter = get_device::<MockPowerMeter>("mock_sensor").await;

    // Readings should be within 2% of base value
    for _ in 0..100 {
        let reading = meter.read().await.unwrap();
        assert!(reading > 0.98e-6 && reading < 1.02e-6);
    }
}
```

## File Structure

```
mock-instruments/
├── plugin.toml      # Plugin manifest
├── mock_stage.toml  # Stage device configurations
├── mock_sensor.toml # Sensor device configurations
└── README.md        # This file
```

## Implementation Reference

Mock implementations are in `crates/hardware/src/drivers/mock.rs`:

- `MockStage`: `crates/hardware/src/drivers/mock.rs:271`
- `MockPowerMeter`: `crates/hardware/src/drivers/mock.rs:827`
- `MockCamera`: `crates/hardware/src/drivers/mock.rs:439`

## License

MIT License - See project root for details.
